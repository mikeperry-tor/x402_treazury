// The deployment startup future includes the HTTP/TLS dependency stack. This
// bounds compiler type traversal only; it does not change runtime work limits.
#![recursion_limit = "256"]

use anyhow::{Context, Result, ensure};
use clap::{CommandFactory, FromArgMatches};
use rmcp::ServiceExt;
use std::{collections::BTreeMap, time::Duration};
use x402_treazury::{
    catalog::{self, Config},
    payment::{PaidClient, Payer, SpendPolicy},
    server::{Server, serve_http},
};

mod cli;
mod stdio;
#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

#[derive(Default)]
struct Args {
    meta_config: Option<std::path::PathBuf>,
    qualification_no_new_funding: bool,
    qualification_unsigned: bool,
    qualification_managed: bool,
    qualification_parent_stdin: bool,
    network_config: Option<std::path::PathBuf>,
    check: bool,
    qualification_snapshot: bool,
    qualification_binding: Option<std::path::PathBuf>,
    show_config: bool,
    config: Option<String>,
    spec: Option<String>,
    base_url: Option<String>,
    prefix: Option<String>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    exclude_tags: Option<Vec<String>>,
    env_file: Option<String>,
    max_api_payment_usdc: Option<String>,
    max_response_bytes: Option<usize>,
    max_help_bytes: Option<usize>,
    max_spec_bytes: Option<usize>,
    read_timeout_seconds: Option<f64>,
    transport: String,
    host: String,
    port: u16,
    bearer_token: Option<String>,
    no_auth: bool,
    allowed_hosts: Option<Vec<String>>,
    disable_host_check: bool,
    list_tools: bool,
    all_tools: bool,
    list_tags: bool,
    route_tool: Option<String>,
    args: String,
}
#[tokio::main]
async fn main() -> std::process::ExitCode {
    finish(run().await)
}
fn finish(result: Result<()>) -> std::process::ExitCode {
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            // Display the cause chain, never anyhow's Debug backtrace.
            eprintln!("error: {error:#}");
            if error.is::<x402_treazury::server::ForcedShutdown>() {
                // The operator explicitly chose force. Returning from tokio::main
                // would still join blocking store/scan/proving work indefinitely.
                // Atomic snapshots and journals remain authoritative on restart;
                // do not run cleanup that releases unfinished liabilities here.
                std::process::exit(1);
            }
            std::process::ExitCode::FAILURE
        }
    }
}
async fn run() -> Result<()> {
    let matches = cli::Cli::command().get_matches();
    let command = cli::Cli::from_arg_matches(&matches)?.command;
    let default_filter = "warn,x402_treazury=info";
    let filter = match std::env::var("RUST_LOG") {
        Ok(value) => {
            tracing_subscriber::EnvFilter::try_new(value).context("invalid RUST_LOG filter")?
        }
        Err(std::env::VarError::NotPresent) => tracing_subscriber::EnvFilter::new(default_filter),
        Err(error) => return Err(error).context("invalid RUST_LOG environment value"),
    };
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .init();
    let args: Args = match command {
        cli::Command::Wallet(args) => return x402_treazury::wallet_cli::run(args).await,
        cli::Command::Sources(cli::Sources::Inspect { config }) => {
            return x402_treazury::discovery::inspect_cli(&config).await;
        }
        cli::Command::Sources(cli::Sources::Remove {
            config,
            server,
            source_id,
        }) => {
            return x402_treazury::discovery::maintain_cli(&config, &server, &source_id, false)
                .await;
        }
        cli::Command::Sources(cli::Sources::Refresh {
            config,
            network_config,
            server,
            source_id,
        }) => {
            let policy = network_config
                .as_deref()
                .map(x402_treazury::network::NetworkConfig::load)
                .transpose()?;
            return x402_treazury::discovery::maintain_cli_with_network(
                &config,
                &server,
                &source_id,
                true,
                policy.as_ref(),
            )
            .await;
        }
        cli::Command::BuildInfo => {
            println!(
                "{}",
                serde_json::to_string_pretty(&x402_treazury::build_identity::current())?
            );
            return Ok(());
        }
        cli::Command::Catalog(cli::Catalog::Warm(args)) => {
            let policy = args
                .network_config
                .as_deref()
                .map(x402_treazury::network::NetworkConfig::load)
                .transpose()?;
            let summary = x402_treazury::deployment::warm_cache_with_network(
                &args.config,
                &args.source,
                args.direct,
                policy.as_ref(),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            return Ok(());
        }
        cli::Command::Serve(args) => args.into(),
        cli::Command::Catalog(args) => args.into(),
        cli::Command::Config(args) => args.into(),
    };
    let definition = cli::Cli::command();
    let mut leaf_definition = &definition;
    let mut matches = &matches;
    while let Some((name, nested)) = matches.subcommand() {
        leaf_definition = leaf_definition
            .find_subcommand(name)
            .expect("parsed command exists");
        matches = nested;
    }
    validate_meta_arguments(&args, matches, leaf_definition)?;
    ensure!(
        !args.no_auth || args.transport == "http",
        "--no-auth requires --transport http"
    );
    ensure!(
        (args.allowed_hosts.is_none() && !args.disable_host_check) || args.transport == "http",
        "--allowed-hosts and --disable-host-check require --transport http"
    );
    x402_treazury::server::host::HostPolicy::new(
        args.allowed_hosts.clone(),
        args.disable_host_check,
    )?;
    let network_override = args
        .network_config
        .as_deref()
        .map(x402_treazury::network::NetworkConfig::load)
        .transpose()?;
    if args.show_config {
        return show_config(&args, network_override.as_ref()).await;
    }
    if args.meta_config.is_none()
        && let Some(policy) = &network_override
    {
        x402_treazury::network::install(policy.network.clone())?;
    }
    let mut env: BTreeMap<String, String> = std::env::vars().collect();
    if let Some(path) = &args.env_file {
        for item in dotenvy::from_path_iter(path)? {
            let (k, v) = item?;
            env.insert(k, v);
        }
    }
    if let Some(path) = &args.meta_config {
        return run_deployment(&args, &env, path, network_override.as_ref()).await;
    }
    run_standalone(args, env).await
}

async fn run_standalone(args: Args, env: BTreeMap<String, String>) -> Result<()> {
    let mut cfg = standalone_config(&args, &env).await?;
    if !args.list_tools && !args.list_tags && !args.check && args.route_tool.is_none() {
        x402_treazury::provider_status::warn(cfg.name.as_deref().unwrap_or("standalone"), &cfg);
    }
    ensure!(
        cfg.read_timeout_seconds
            .map(Duration::from_secs_f64)
            .is_none_or(|t| t >= Duration::from_millis(1)),
        "HTTP read inactivity timeout must be at least 1ms"
    );
    if args.all_tools {
        cfg.use_recommended_tools = false;
    }
    let root = catalog::load_json_discovery(&cfg).await?;
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
    cfg.resolve_cover(
        &base,
        x402_treazury::network::global().policy.cover_enabled(),
    )?;
    let prefix = cfg
        .prefix
        .clone()
        .unwrap_or(catalog::default_prefix(&base)?);
    x402_treazury::config::validate(&cfg)?;
    let tools = catalog::build_tools(&cfg, &root, &prefix)?;
    tracing::info!(target: "x402_treazury::startup", tools = tools.len(), "Catalog ready");
    if inspect_standalone(&args, &cfg, &base, &tools).await? {
        return Ok(());
    }
    let server = prepare_standalone_server(&args, &env, cfg, base, tools).await?;
    if args.transport == "stdio" {
        serve_standalone_stdio(server).await?;
    } else {
        let token = if args.no_auth {
            None
        } else {
            Some(args
            .bearer_token
            .or_else(|| env.get("X402_MCP_BEARER_TOKEN").cloned())
            .filter(|s| !s.is_empty())
            .context("HTTP transport requires X402_MCP_BEARER_TOKEN or --bearer-token; use --no-auth to explicitly disable authentication")?)
        };
        let listener = tokio::net::TcpListener::bind((args.host.as_str(), args.port)).await?;
        tracing::info!(address = %listener.local_addr()?, "MCP listening at /mcp");
        serve_http(listener, server, token, shutdown_signal()).await?;
    }
    Ok(())
}

async fn inspect_standalone(
    args: &Args,
    cfg: &Config,
    base: &str,
    tools: &[catalog::ToolSpec],
) -> Result<bool> {
    if args.list_tools {
        let recommended = cfg.recommendation_matcher()?;
        let inventory: Vec<_> = tools
            .iter()
            .map(|tool| catalog::ToolInspection {
                tool,
                recommended: recommended(&tool.name),
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&inventory)?);
        return Ok(true);
    }
    if let Some(name) = &args.route_tool {
        let tool = tools
            .iter()
            .find(|t| t.name == *name)
            .context("unknown tool")?;
        println!(
            "{}",
            serde_json::to_string_pretty(&tool.route(base, &serde_json::from_str(&args.args)?)?)?
        );
        return Ok(true);
    }
    if args.check {
        println!("{} tools validated", tools.len());
        return Ok(true);
    }
    Ok(false)
}

async fn prepare_standalone_server(
    args: &Args,
    env: &BTreeMap<String, String>,
    cfg: Config,
    base: String,
    tools: Vec<catalog::ToolSpec>,
) -> Result<Server> {
    let policy = SpendPolicy::dollars(
        args.max_api_payment_usdc
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
        PaidClient::new(payer)
            .with_transport(cfg.transport())
            .with_cover(
                cfg.cover_traffic.clone(),
                x402_treazury::cover::Scope {
                    listener: "standalone".into(),
                    source: "standalone".into(),
                },
            )
            .with_timeout(cfg.read_timeout_seconds.map(Duration::from_secs_f64))
            .with_download_limits(cfg.max_response_bytes, cfg.max_help_bytes),
        base,
        cfg.instructions_text,
    );
    if let Some(name) = cfg.name {
        server.name = name;
    }
    server.host_policy = x402_treazury::server::host::HostPolicy::new(
        args.allowed_hosts.clone(),
        args.disable_host_check,
    )?;
    Ok(server)
}

async fn serve_standalone_stdio(server: Server) -> Result<()> {
    let work = server.work.clone();
    let service = server.serve(stdio::transport()?).await?;
    let cancel = service.cancellation_token();
    let waiting = service.waiting();
    tokio::pin!(waiting);
    let result = tokio::select! {
        result = &mut waiting => Some(result),
        signal = shutdown_signal() => { signal?; None }
    };
    work.close();
    cancel.cancel();
    tracing::info!("Stdio stopping: draining accepted calls; signal to force exit");
    if let Some(engine) = &x402_treazury::network::global().cover {
        engine.stop_ranges().await;
    }
    let result = tokio::select! {
        result = async {
            let (result, ()) = tokio::join!(async {
                match result { Some(result) => result, None => waiting.await }
            }, work.drain());
            result
        } => result,
        signal = x402_treazury::server::force_signal() => { signal?; anyhow::bail!(x402_treazury::server::ForcedShutdown); }
    };
    result?;
    if let Some(engine) = &x402_treazury::network::global().cover {
        engine.shutdown().await;
    }
    Ok(())
}

fn validate_meta_arguments(
    args: &Args,
    matches: &clap::ArgMatches,
    command: &clap::Command,
) -> Result<()> {
    ensure!(
        args.network_config.is_none()
            || !(args.qualification_snapshot
                || args.qualification_unsigned
                || args.qualification_managed
                || args.qualification_parent_stdin
                || args.qualification_binding.is_some()),
        "qualification requires network policy in the pinned deployment; --network-config is unavailable"
    );
    if args.meta_config.is_some() {
        for argument in command.get_arguments() {
            let id = argument.get_id();
            if matches.value_source(id.as_str()) == Some(clap::parser::ValueSource::CommandLine) {
                ensure!(
                    [
                        "meta_config",
                        "network_config",
                        "all_tools",
                        "env_file",
                        "qualification_no_new_funding",
                        "qualification_unsigned",
                        "qualification_managed",
                        "qualification_parent_stdin",
                        "qualification_snapshot",
                        "qualification_binding"
                    ]
                    .contains(&id.as_str()),
                    "--config cannot be combined with --{}; configure it in the TOML file",
                    if id.as_str() == "config" {
                        "provider".to_owned()
                    } else {
                        id.as_str().replace('_', "-")
                    }
                );
            }
        }
    }
    Ok(())
}

async fn show_config(
    args: &Args,
    network_override: Option<&x402_treazury::network::NetworkConfig>,
) -> Result<()> {
    let mut value = if let Some(path) = &args.meta_config {
        x402_treazury::deployment::Deployment::show_config_with_network(path, network_override)
            .await?
    } else {
        let path = args
            .config
            .as_ref()
            .context("config show requires --provider or --config")?;
        serde_json::to_value(x402_treazury::config::load(std::path::Path::new(path)).await?)?
    };
    if args.meta_config.is_none() {
        value["network"] = network_override
            .map(|config| &config.network)
            .cloned()
            .unwrap_or_default()
            .inspection();
    }
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

async fn run_deployment(
    args: &Args,
    env: &BTreeMap<String, String>,
    path: &std::path::Path,
    network_override: Option<&x402_treazury::network::NetworkConfig>,
) -> Result<()> {
    let mut parent =
        x402_treazury::supervision::Parent::from_stdin(args.qualification_parent_stdin)?;
    if let Some(binding) = &args.qualification_binding {
        ensure!(
            args.qualification_unsigned || args.qualification_managed,
            "qualification binding requires an explicit unsigned or managed mode"
        );
        if args.qualification_managed {
            x402_treazury::qualification::install_managed(binding, path)?;
        } else {
            x402_treazury::qualification::install(binding, path)?;
        }
    }
    if args.qualification_snapshot {
        return inspect_deployment_snapshot(path, &mut parent).await;
    }
    let loading = load_deployment(args, env, path, network_override);
    let mut deployment = tokio::select! {
        result = loading => result?,
        closed = parent.closed() => { closed?; anyhow::bail!("qualification supervisor closed during catalog startup; no server started"); }
    };
    if print_deployment_inspection(args, &mut deployment).await? {
        return Ok(());
    }
    let restriction = if args.qualification_no_new_funding {
        x402_treazury::rotation::restriction::FundingRestriction::DenyNewFunding
    } else {
        Default::default()
    };
    let binding = async {
        if args.qualification_unsigned {
            deployment.bind_unsigned(env).await
        } else {
            deployment.bind_restricted(env, restriction).await
        }
    };
    let running = tokio::select! {
        result = binding => result?,
        closed = parent.closed() => { closed?; anyhow::bail!("qualification supervisor closed during binding; startup cancelled"); }
    };
    serve_deployment(running, &mut parent).await
}

async fn inspect_deployment_snapshot(
    path: &std::path::Path,
    parent: &mut x402_treazury::supervision::Parent,
) -> Result<()> {
    let (result, stages) = tokio::select! {
        result = x402_treazury::deployment::Deployment::inspect_catalogs(path) => result,
        closed = parent.closed() => { closed?; anyhow::bail!("qualification supervisor closed during catalog inspection"); }
    };
    let mut snapshot = match &result {
        Ok(deployment) => deployment.qualification_snapshot(),
        Err(_) => serde_json::json!({"version":1,"preparation_failed":true}),
    };
    snapshot["catalog_stages"] = serde_json::to_value(stages)?;
    let bytes = serde_json::to_vec(&snapshot)?;
    ensure!(
        bytes.len() < x402_treazury::deployment::QUALIFICATION_SNAPSHOT_BYTES,
        "qualification snapshot exceeds {}-byte limit including newline; split the deployment into smaller qualification runs",
        x402_treazury::deployment::QUALIFICATION_SNAPSHOT_BYTES
    );
    println!("{}", std::str::from_utf8(&bytes)?);
    result?;
    Ok(())
}

async fn load_deployment(
    args: &Args,
    env: &BTreeMap<String, String>,
    path: &std::path::Path,
    network_override: Option<&x402_treazury::network::NetworkConfig>,
) -> Result<x402_treazury::deployment::Deployment> {
    if args.list_tags || args.list_tools || args.check || args.qualification_snapshot {
        x402_treazury::deployment::Deployment::load_for_inspection_with_network(
            path,
            args.all_tools,
            network_override,
        )
        .await
    } else if args.qualification_unsigned {
        x402_treazury::deployment::Deployment::load_for_unsigned_serving(path).await
    } else {
        x402_treazury::deployment::Deployment::load_for_serving_with_relay_and_network(
            path,
            env,
            // Supervised startup can be cancelled by parent EOF. Keep its
            // funding lifecycle in the existing post-bind supervisor; ordinary
            // startup bootstrap handles signals and drains before returning.
            if args.qualification_no_new_funding || args.qualification_parent_stdin {
                x402_treazury::rotation::restriction::FundingRestriction::DenyNewFunding
            } else {
                Default::default()
            },
            network_override,
        )
        .await
    }
}

async fn print_deployment_inspection(
    args: &Args,
    deployment: &mut x402_treazury::deployment::Deployment,
) -> Result<bool> {
    if args.list_tags {
        println!(
            "{}",
            serde_json::to_string_pretty(&deployment.tag_inventory()?)?
        );
        return Ok(true);
    }
    if args.list_tools {
        println!("{}", serde_json::to_string_pretty(&deployment.inventory())?);
        return Ok(true);
    }
    if args.check {
        let summary = deployment.wallet_summary();
        println!(
            "{} managed pools ({} automatic); initial inventory target: {} USDC",
            summary.managed_pool_count,
            summary.generated_pool_count,
            summary.initial_inventory_target_usdc
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
        return Ok(true);
    }
    Ok(false)
}

async fn serve_deployment(
    running: x402_treazury::deployment::RunningDeployment,
    parent: &mut x402_treazury::supervision::Parent,
) -> Result<()> {
    let shutdown = tokio_util::sync::CancellationToken::new();
    let serving = running.serve(shutdown.clone());
    tokio::pin!(serving);
    let result = tokio::select! {
        result = &mut serving => result,
        closed = parent.closed() => {
            shutdown.cancel();
            let result = tokio::select! {
                result = &mut serving => result,
                force = x402_treazury::server::force_signal() => { force?; anyhow::bail!(x402_treazury::server::ForcedShutdown); }
            };
            closed?;
            result
        }
        signal = shutdown_signal() => {
            shutdown.cancel();
            let result = tokio::select! {
                result = &mut serving => result,
                force = x402_treazury::server::force_signal() => {
                    force?;
                    anyhow::bail!(x402_treazury::server::ForcedShutdown);
                }
            };
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
    if let Some(timeout) = args.read_timeout_seconds {
        cfg.read_timeout_seconds = Some(timeout);
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
