use anyhow::{Context, Result, ensure};
use clap::{CommandFactory, FromArgMatches, Parser};
use rmcp::ServiceExt;
use std::{collections::BTreeMap, time::Duration};
use x402_mcp_prototype::{
    catalog::{self, Config},
    payment::{PaidClient, Payer, SpendPolicy},
    server::{Server, http_app},
};

#[derive(Parser)]
#[command(about = "Experimental Rust generic x402 MCP server")]
struct Args {
    #[arg(long)]
    meta_config: Option<std::path::PathBuf>,
    #[arg(long, requires = "meta_config")]
    check: bool,
    #[arg(long)]
    config: Option<String>,
    #[arg(long)]
    spec: Option<String>,
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long)]
    prefix: Option<String>,
    #[arg(long, value_delimiter = ',')]
    include: Option<Vec<String>>,
    #[arg(long, value_delimiter = ',')]
    exclude: Option<Vec<String>>,
    #[arg(long, value_delimiter = ',')]
    tags: Option<Vec<String>>,
    #[arg(long, value_delimiter = ',')]
    exclude_tags: Option<Vec<String>>,
    #[arg(long)]
    env_file: Option<String>,
    #[arg(long)]
    max_price_usd: Option<String>,
    #[arg(long)]
    max_response_chars: Option<usize>,
    #[arg(long, default_value = "30")]
    timeout: f64,
    #[arg(long, default_value="stdio", value_parser=["stdio", "http"])]
    transport: String,
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value = "8000")]
    port: u16,
    #[arg(long)]
    bearer_token: Option<String>,
    #[arg(long)]
    list_tools: bool,
    #[arg(long)]
    route_tool: Option<String>,
    #[arg(long, default_value = "{}")]
    args: String,
}
#[tokio::main]
async fn main() -> Result<()> {
    let matches = Args::command().get_matches();
    let args = Args::from_arg_matches(&matches)?;
    if args.meta_config.is_some() {
        for argument in Args::command().get_arguments() {
            let id = argument.get_id();
            if matches.value_source(id.as_str()) == Some(clap::parser::ValueSource::CommandLine) {
                ensure!(
                    ["meta_config", "check", "list_tools", "env_file"].contains(&id.as_str()),
                    "--meta-config cannot be combined with --{}; configure it in the TOML file",
                    id.as_str().replace('_', "-")
                );
            }
        }
    }
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter("warn")
        .init();
    let mut env: BTreeMap<String, String> = std::env::vars().collect();
    if let Some(path) = &args.env_file {
        for item in dotenvy::from_path_iter(path)? {
            let (k, v) = item?;
            env.insert(k, v);
        }
    }
    if let Some(path) = &args.meta_config {
        let deployment = x402_mcp_prototype::deployment::Deployment::load(path).await?;
        if args.list_tools {
            println!("{}", serde_json::to_string_pretty(&deployment.inventory())?);
            return Ok(());
        }
        if args.check {
            for server in deployment.inventory() {
                println!(
                    "{}: {} tools on {} (wallet {})",
                    server.server,
                    server.tools.len(),
                    server.listen,
                    server.wallet
                );
            }
            return Ok(());
        }
        let running = deployment.bind(&env).await?;
        for (server, address) in running.addresses() {
            tracing::warn!(server, %address, "MCP listening at /mcp");
        }
        let shutdown = tokio_util::sync::CancellationToken::new();
        let serving = running.serve(shutdown.clone());
        tokio::pin!(serving);
        let result = tokio::select! {
            result = &mut serving => result,
            signal = shutdown_signal() => {
                shutdown.cancel();
                let result = serving.await;
                signal.context("shutdown signal handler failed")?;
                result
            }
        };
        return result;
    }
    let mut cfg: Config = if let Some(path) = args.config {
        serde_json::from_slice(&tokio::fs::read(path).await?)?
    } else {
        Config::default()
    };
    // Same precedence as Python for the supported generic settings.
    macro_rules! string_field {
        ($field:ident, $key:literal) => {
            if let Some(value) = env.get($key) {
                cfg.$field = Some(value.clone());
            }
        };
    }
    if let Some(value) = env.get("X402_MCP_GENERIC_SPEC") {
        cfg.spec = value.clone();
    }
    string_field!(base_url, "X402_MCP_GENERIC_BASE_URL");
    string_field!(prefix, "X402_MCP_GENERIC_PREFIX");
    string_field!(name, "X402_MCP_GENERIC_NAME");
    string_field!(pricing_key, "X402_MCP_GENERIC_PRICING_KEY");
    if let Some(value) = env.get("X402_MCP_GENERIC_MAX_DESCRIPTION_CHARS") {
        cfg.max_description_chars = Some(value.parse().context("invalid max_description_chars")?);
    }
    ensure!(
        cfg.max_description_chars != Some(0),
        "max_description_chars must be positive"
    );
    string_field!(instructions_text, "X402_MCP_GENERIC_INSTRUCTIONS_TEXT");
    string_field!(help_url, "X402_MCP_GENERIC_HELP_URL");
    macro_rules! filter {
        ($field:ident, $key:literal) => {
            if let Some(value) = env.get($key) {
                cfg.$field = value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            if let Some(value) = args.$field {
                cfg.$field = value;
            }
        };
    }
    filter!(include, "X402_MCP_GENERIC_INCLUDE");
    filter!(exclude, "X402_MCP_GENERIC_EXCLUDE");
    filter!(tags, "X402_MCP_GENERIC_TAGS");
    filter!(exclude_tags, "X402_MCP_GENERIC_EXCLUDE_TAGS");
    if let Some(spec) = args.spec {
        cfg.spec = spec;
    }
    if let Some(base) = args.base_url {
        cfg.base_url = Some(base);
    }
    if let Some(prefix) = args.prefix {
        cfg.prefix = Some(prefix);
    }
    ensure!(!cfg.spec.is_empty(), "--spec or config spec is required");
    ensure!(
        args.timeout.is_finite() && args.timeout > 0.0,
        "timeout must be positive"
    );
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs_f64(args.timeout))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let root = catalog::load_json(&cfg.spec, &http).await?;
    let base = cfg
        .base_url
        .clone()
        .or_else(|| {
            root.pointer("/servers/0/url")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
        .context("base_url is required for digests/specs without servers")?;
    let prefix = cfg
        .prefix
        .clone()
        .unwrap_or(catalog::default_prefix(&base)?);
    let tools = catalog::build_tools(&cfg, &root, &prefix)?;
    if args.list_tools {
        println!("{}", serde_json::to_string_pretty(&tools)?);
        return Ok(());
    }
    if let Some(name) = args.route_tool {
        let tool = tools
            .iter()
            .find(|t| t.name == name)
            .context("unknown tool")?;
        println!(
            "{}",
            serde_json::to_string_pretty(&tool.route(&base, &serde_json::from_str(&args.args)?)?)?
        );
        return Ok(());
    }
    if cfg.probe_pricing {
        tracing::warn!("Rust prototype does not probe prices; descriptions use spec prices only");
    }
    let policy = SpendPolicy::dollars(
        args.max_price_usd
            .as_deref()
            .or_else(|| env.get("X402_MAX_PRICE_USD").map(String::as_str))
            .unwrap_or("1.00"),
    )?;
    let payer = Payer::new(
        env.get("EVM_PRIVATE_KEY")
            .context("EVM_PRIVATE_KEY required")?,
        policy,
    )?;
    let mut server = Server::new(
        tools,
        PaidClient::new(http, payer),
        base,
        cfg.instructions_text,
        args.max_response_chars,
    );
    if let Some(name) = cfg.name {
        server.name = name;
    }
    if args.transport == "stdio" {
        server
            .serve(rmcp::transport::stdio())
            .await?
            .waiting()
            .await?;
    } else {
        let token = args
            .bearer_token
            .or_else(|| env.get("X402_MCP_BEARER_TOKEN").cloned())
            .filter(|s| !s.is_empty())
            .context("HTTP transport requires X402_MCP_BEARER_TOKEN or --bearer-token")?;
        let listener = tokio::net::TcpListener::bind((args.host.as_str(), args.port)).await?;
        tracing::warn!(address = %listener.local_addr()?, "MCP listening at /mcp");
        axum::serve(listener, http_app(server, token))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?;
    }
    Ok(())
}

async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! { result = tokio::signal::ctrl_c() => result?, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}
