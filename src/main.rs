use anyhow::{Context, Result, ensure};
use clap::{CommandFactory, FromArgMatches, Parser};
use rmcp::ServiceExt;
use std::{collections::BTreeMap, time::Duration};
use x402_treazury::{
    catalog::{self, Config},
    payment::{PaidClient, Payer, SpendPolicy},
    server::{Server, serve_http},
};

#[derive(Parser)]
#[command(
    name = "treazury",
    version,
    about = "x402_treazury: paid API tools with managed wallets and optional Tor isolation",
    after_help = "Treasury commands: wallet --help (init/addresses/address/pool require the zcash feature)"
)]
struct Args {
    #[arg(long)]
    meta_config: Option<std::path::PathBuf>,
    #[arg(long, conflicts_with = "meta_config")]
    network_config: Option<std::path::PathBuf>,
    #[arg(long, requires = "meta_config")]
    check: bool,
    #[arg(long, conflicts_with_all = ["check", "list_tools", "list_tags", "route_tool"])]
    show_config: bool,
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
    #[arg(long)]
    max_response_bytes: Option<usize>,
    #[arg(long)]
    max_help_bytes: Option<usize>,
    #[arg(long)]
    max_spec_bytes: Option<usize>,
    #[arg(long)]
    timeout: Option<f64>,
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
    #[arg(long, conflicts_with_all = ["list_tools", "check", "route_tool"])]
    list_tags: bool,
    #[arg(long)]
    route_tool: Option<String>,
    #[arg(long, default_value = "{}")]
    args: String,
}
#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            // Display the cause chain, never anyhow's Debug backtrace.
            eprintln!("error: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
async fn run() -> Result<()> {
    if std::env::args().nth(1).as_deref() == Some("wallet") {
        return x402_treazury::wallet_cli::run().await;
    }
    if std::env::args().nth(1).as_deref() == Some("sources") {
        return x402_treazury::discovery::inspect_cli().await;
    }
    let matches = Args::command().get_matches();
    let args = Args::from_arg_matches(&matches)?;
    validate_meta_arguments(&args, &matches)?;
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter("warn,x402_treazury::startup=info,x402_treazury::network=info")
        .init();
    if args.show_config {
        return show_config(&args, &matches).await;
    }
    if let Some(path) = &args.network_config {
        x402_treazury::network::install(x402_treazury::network::NetworkPolicy::load(path)?)?;
    }
    let mut env: BTreeMap<String, String> = std::env::vars().collect();
    if let Some(path) = &args.env_file {
        for item in dotenvy::from_path_iter(path)? {
            let (k, v) = item?;
            env.insert(k, v);
        }
    }
    if let Some(path) = &args.meta_config {
        return run_deployment(&args, &env, path).await;
    }
    run_standalone(args, env).await
}

async fn run_standalone(args: Args, env: BTreeMap<String, String>) -> Result<()> {
    let cfg = standalone_config(&args, &env).await?;
    if !args.list_tools && !args.list_tags && !args.check && args.route_tool.is_none() {
        x402_treazury::provider_status::warn(cfg.name.as_deref().unwrap_or("standalone"), &cfg);
    }
    let http = x402_treazury::network::provider_discovery(
        if cfg.spec.starts_with("http") {
            &cfg.spec
        } else {
            "https://local.invalid"
        },
        Duration::from_secs_f64(cfg.timeout),
        cfg.transport(),
    )?;
    let root = catalog::load_json_with_limit(&cfg.spec, &http, cfg.max_spec_bytes).await?;
    if args.list_tags {
        println!(
            "{}",
            serde_json::to_string_pretty(&catalog::tag_counts(&root)?)?
        );
        return Ok(());
    }
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
    x402_treazury::pricing::validate(&cfg)?;
    let mut tools = catalog::build_tools(&cfg, &root, &prefix)?;
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
    let prices = x402_treazury::pricing::process_cache()
        .discover(&cfg, &root, &tools, &base)
        .await?;
    tools = catalog::build_tools_with_prices(&cfg, &root, &prefix, &prices)?;
    let mut server = Server::new(
        tools,
        PaidClient::new(payer)
            .with_transport(cfg.transport())
            .with_timeout(Duration::from_secs_f64(cfg.timeout))
            .with_download_limits(cfg.max_response_bytes, cfg.max_help_bytes),
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
        serve_http(listener, server, token, shutdown_signal()).await?;
    }
    Ok(())
}

fn validate_meta_arguments(args: &Args, matches: &clap::ArgMatches) -> Result<()> {
    if args.meta_config.is_some() {
        for argument in Args::command().get_arguments() {
            let id = argument.get_id();
            if matches.value_source(id.as_str()) == Some(clap::parser::ValueSource::CommandLine) {
                ensure!(
                    [
                        "meta_config",
                        "check",
                        "show_config",
                        "list_tools",
                        "list_tags",
                        "env_file"
                    ]
                    .contains(&id.as_str()),
                    "--meta-config cannot be combined with --{}; configure it in the TOML file",
                    id.as_str().replace('_', "-")
                );
            }
        }
    }
    Ok(())
}

async fn show_config(args: &Args, matches: &clap::ArgMatches) -> Result<()> {
    for argument in Args::command().get_arguments() {
        let id = argument.get_id();
        if matches.value_source(id.as_str()) == Some(clap::parser::ValueSource::CommandLine) {
            ensure!(
                ["show_config", "config", "meta_config", "network_config"].contains(&id.as_str()),
                "--show-config resolves configuration files only; omit --{}",
                id.as_str().replace('_', "-")
            );
        }
    }
    let mut value = if let Some(path) = &args.meta_config {
        x402_treazury::deployment::Deployment::show_config(path).await?
    } else {
        let path = args
            .config
            .as_ref()
            .context("--show-config requires --config or --meta-config")?;
        serde_json::to_value(x402_treazury::config::load(std::path::Path::new(path)).await?)?
    };
    if args.meta_config.is_none() {
        let policy = args
            .network_config
            .as_ref()
            .map(|path| x402_treazury::network::NetworkPolicy::load(path))
            .transpose()?
            .unwrap_or_default();
        value["network"] = policy.inspection();
    }
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

async fn run_deployment(
    args: &Args,
    env: &BTreeMap<String, String>,
    path: &std::path::Path,
) -> Result<()> {
    let deployment = if args.list_tags || args.list_tools || args.check {
        x402_treazury::deployment::Deployment::load(path).await?
    } else {
        x402_treazury::deployment::Deployment::load_for_serving(path).await?
    };
    if args.list_tags {
        println!(
            "{}",
            serde_json::to_string_pretty(&deployment.tag_inventory()?)?
        );
        return Ok(());
    }
    if args.list_tools {
        println!("{}", serde_json::to_string_pretty(&deployment.inventory())?);
        return Ok(());
    }
    if args.check {
        let summary = deployment.wallet_summary();
        println!(
            "{} managed pools ({} automatic); active + standby target: {} USDC",
            summary.managed_pool_count,
            summary.generated_pool_count,
            summary.active_and_standby_target_usdc
        );
        for server in deployment.inventory() {
            println!(
                "{}: {} tools on {}",
                server.server,
                server.tools.len(),
                server.listen
            );
            if !server.management_tools.is_empty() {
                println!(
                    "  {} source-management/fallback tools",
                    server.management_tools.len()
                );
            }
            for (source, binding) in server.wallet_bindings {
                println!("  {source}: wallet {} ({})", binding.wallet, binding.origin);
            }
        }
        return Ok(());
    }
    let running = deployment.bind(env).await?;
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
    result
}

async fn standalone_config(args: &Args, env: &BTreeMap<String, String>) -> Result<Config> {
    let mut cfg: Config = if let Some(path) = &args.config {
        let resolved = x402_treazury::config::load(std::path::Path::new(path)).await?;
        resolved.settings
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
            if let Some(value) = &args.$field {
                cfg.$field = value.clone();
            }
        };
    }
    filter!(include, "X402_MCP_GENERIC_INCLUDE");
    filter!(exclude, "X402_MCP_GENERIC_EXCLUDE");
    filter!(tags, "X402_MCP_GENERIC_TAGS");
    filter!(exclude_tags, "X402_MCP_GENERIC_EXCLUDE_TAGS");
    if let Some(spec) = &args.spec {
        cfg.spec = spec.clone();
    }
    if let Some(base) = &args.base_url {
        cfg.base_url = Some(base.clone());
    }
    if let Some(prefix) = &args.prefix {
        cfg.prefix = Some(prefix.clone());
    }
    ensure!(!cfg.spec.is_empty(), "--spec or config spec is required");
    if let Some(timeout) = args.timeout {
        cfg.timeout = timeout;
    }
    if let Some(value) = args.max_response_bytes {
        cfg.max_response_bytes = value;
    }
    if let Some(value) = args.max_help_bytes {
        cfg.max_help_bytes = value;
    }
    if let Some(value) = args.max_spec_bytes {
        cfg.max_spec_bytes = value;
    }
    x402_treazury::config::validate(&cfg)?;
    Ok(cfg)
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
