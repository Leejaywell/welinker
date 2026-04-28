use crate::{
    agent::{
        AcpAgent, AcpAgentConfig, CliAgent, CliAgentConfig, HttpAgent, HttpAgentConfig, SharedAgent,
    },
    api::Server,
    config::{self, AgentConfig},
    ilink::{self, Client, Monitor},
    messaging::{
        send_media_from_url, send_text_reply, AgentFactory, AgentMeta, Handler, SaveDefault,
    },
};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use futures::FutureExt;
use nix::{
    sys::signal::{kill, Signal},
    unistd::Pid,
};
use qrcode::{render::unicode, QrCode};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{oneshot, RwLock};
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Debug, Parser)]
#[command(
    name = "welinker",
    version,
    about = "WeChat AI agent bridge implemented in Rust"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    #[command(about = "Start the WeChat message bridge")]
    Start {
        #[arg(short, long)]
        foreground: bool,
        #[arg(long, default_value = "")]
        api_addr: String,
        #[arg(long)]
        web_only: bool,
    },
    #[command(about = "Add a WeChat account via QR code scan")]
    Login,
    #[command(about = "Send a message to a WeChat user")]
    Send {
        #[arg(long)]
        to: String,
        #[arg(long, default_value = "")]
        text: String,
        #[arg(long, default_value = "")]
        media: String,
        #[arg(long, default_value = "")]
        account: String,
    },
    #[command(subcommand, about = "Manage WeChat accounts")]
    Accounts(AccountCommands),
    #[command(about = "Check whether welinker is running")]
    Status,
    #[command(about = "Stop the background process")]
    Stop,
    #[command(about = "Restart the background process")]
    Restart,
    #[command(about = "Print current version")]
    Version,
}

#[derive(Debug, Subcommand)]
enum AccountCommands {
    #[command(about = "List saved WeChat accounts")]
    List,
    #[command(about = "Remove a saved WeChat account")]
    Remove { account: String },
}

pub async fn execute() -> Result<()> {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Commands::Start {
        foreground: false,
        api_addr: String::new(),
        web_only: false,
    });
    match command {
        Commands::Start {
            foreground,
            api_addr,
            web_only,
        } => run_start(foreground, api_addr, web_only).await,
        Commands::Login => {
            init_logging(false)?;
            let creds = do_login().await?;
            println!(
                "Account {} added. Run 'welinker start' to begin.",
                creds.ilink_bot_id
            );
            Ok(())
        }
        Commands::Send {
            to,
            text,
            media,
            account,
        } => {
            init_logging(false)?;
            run_send(to, text, media, account).await
        }
        Commands::Accounts(command) => {
            init_logging(false)?;
            run_accounts(command)
        }
        Commands::Status => {
            run_status();
            Ok(())
        }
        Commands::Stop => {
            stop_all();
            println!("welinker stopped");
            Ok(())
        }
        Commands::Restart => {
            stop_all();
            tokio::time::sleep(Duration::from_millis(500)).await;
            run_daemon(false)
        }
        Commands::Version => {
            println!(
                "welinker {} ({}/{})",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            Ok(())
        }
    }
}

async fn run_start(foreground: bool, api_addr: String, web_only: bool) -> Result<()> {
    if !foreground {
        let accounts = ilink::load_all_credentials()?;
        if accounts.is_empty() && !web_only {
            init_logging(false)?;
            println!("No WeChat accounts found, starting login...");
            do_login().await?;
        }
        return run_daemon(web_only);
    }

    init_logging(true)?;
    let mut accounts = ilink::load_all_credentials()?;
    if accounts.is_empty() && !web_only {
        tracing::info!("No WeChat accounts found, starting login...");
        accounts.push(do_login().await?);
    } else if accounts.is_empty() {
        tracing::info!("No WeChat accounts found, starting WebUI without iLink clients");
    }

    let mut cfg = config::load()?;
    if config::detect_and_configure(&mut cfg) {
        config::save(&cfg)?;
        tracing::info!(path = %config::config_path().display(), "auto-detected agents saved");
    }
    if !api_addr.is_empty() {
        cfg.api_addr = api_addr;
    }

    let cfg = Arc::new(RwLock::new(cfg));
    let factory_cfg = Arc::clone(&cfg);
    let factory: AgentFactory = Arc::new(move |name: String| {
        let cfg = Arc::clone(&factory_cfg);
        async move {
            let ag = cfg.read().await.agents.get(&name).cloned();
            create_agent_by_config(&name, ag?).await
        }
        .boxed()
    });
    let save_cfg = Arc::clone(&cfg);
    let save_default: SaveDefault = Arc::new(move |name: String| {
        let cfg = Arc::clone(&save_cfg);
        async move {
            let mut cfg = cfg.write().await;
            cfg.default_agent = name;
            config::save(&cfg)
        }
        .boxed()
    });
    let handler = Handler::new(factory, save_default);

    {
        let cfg_read = cfg.read().await;
        let metas = cfg_read
            .agents
            .iter()
            .map(|(name, ag)| AgentMeta {
                name: name.clone(),
                kind: ag.kind.clone(),
                command: if ag.kind == "http" {
                    ag.endpoint.clone()
                } else {
                    ag.command.clone()
                },
                model: ag.model.clone(),
            })
            .collect::<Vec<_>>();
        let work_dirs = cfg_read
            .agents
            .iter()
            .filter_map(|(name, ag)| {
                if ag.cwd.is_empty() {
                    None
                } else {
                    Some((name.clone(), PathBuf::from(&ag.cwd)))
                }
            })
            .collect::<HashMap<_, _>>();
        handler.set_agent_metas(metas).await;
        handler.set_agent_work_dirs(work_dirs).await;
        handler
            .set_custom_aliases(config::build_alias_map(&cfg_read.agents))
            .await;
        if !cfg_read.save_dir.is_empty() {
            handler
                .set_save_dir(Some(PathBuf::from(&cfg_read.save_dir)))
                .await;
        }
    }

    let default_name = cfg.read().await.default_agent.clone();
    if !default_name.is_empty() {
        handler.set_default_agent_name(default_name.clone()).await;
        tracing::info!(agent = default_name, "default agent selected");
    }

    let cfg_read = cfg.read().await;
    let route_tag = cfg_read.route_tag.clone();
    let clients = accounts
        .iter()
        .map(|creds| Client::new_with_route_tag(creds, Some(route_tag.clone())))
        .collect::<Vec<_>>();
    let api_addr = cfg_read.api_addr.clone();
    drop(cfg_read);
    let api_clients = clients.clone();
    let api_handler = Arc::clone(&handler);
    tokio::spawn(async move {
        if let Err(err) = Server::new(api_clients, api_addr)
            .with_handler(api_handler)
            .run()
            .await
        {
            tracing::error!(error = %err, "api server stopped");
        }
    });

    let mut shutdown_senders = Vec::new();
    for client in clients {
        let handler = Arc::clone(&handler);
        let callback: ilink::MessageHandler = Arc::new(move |client, msg| {
            let handler = Arc::clone(&handler);
            async move {
                handler.handle_message(client, msg).await;
            }
            .boxed()
        });
        let monitor = Monitor::new(client, callback)?;
        let (tx, rx) = oneshot::channel();
        shutdown_senders.push(tx);
        tokio::spawn(async move {
            if let Err(err) = monitor.run(rx).await {
                tracing::warn!(error = %err, "monitor stopped");
            }
        });
    }

    tracing::info!("message bridge started");
    tokio::signal::ctrl_c().await?;
    for tx in shutdown_senders {
        let _ = tx.send(());
    }
    Ok(())
}

async fn create_agent_by_config(name: &str, ag: AgentConfig) -> Option<SharedAgent> {
    let cwd = if ag.cwd.is_empty() {
        config::default_workspace()
    } else {
        PathBuf::from(&ag.cwd)
    };
    match ag.kind.as_str() {
        "acp" => {
            let agent = Arc::new(AcpAgent::new(AcpAgentConfig {
                command: ag.command.clone(),
                args: ag.args.clone(),
                cwd,
                env: ag.env.clone(),
                model: ag.model.clone(),
                system_prompt: ag.system_prompt.clone(),
            }));
            if let Err(err) = agent.start().await {
                tracing::warn!(agent = name, error = %err, "failed to start ACP agent");
                None
            } else {
                Some(agent)
            }
        }
        "cli" => Some(Arc::new(CliAgent::new(CliAgentConfig {
            name: name.to_string(),
            command: ag.command.clone(),
            args: ag.args.clone(),
            cwd,
            env: ag.env.clone(),
            model: ag.model.clone(),
            system_prompt: ag.system_prompt.clone(),
        }))),
        "http" => Some(Arc::new(HttpAgent::new(HttpAgentConfig {
            endpoint: ag.endpoint.clone(),
            api_key: ag.api_key.clone(),
            headers: ag.headers.clone(),
            model: ag.model.clone(),
            system_prompt: ag.system_prompt.clone(),
            max_history: ag.max_history,
        }))),
        _ => {
            tracing::warn!(agent = name, kind = ag.kind, "unknown agent type");
            None
        }
    }
}

async fn do_login() -> Result<ilink::Credentials> {
    println!("Fetching QR code...");
    let qr = ilink::fetch_qrcode().await?;
    println!("\nScan this QR code with WeChat:\n");
    if let Ok(code) = QrCode::new(qr.qrcode_img_content.as_bytes()) {
        println!(
            "{}",
            code.render::<unicode::Dense1x2>().quiet_zone(true).build()
        );
    }
    println!("\nQR URL: {}\n\nWaiting for scan...", qr.qrcode_img_content);
    let mut last_status = String::new();
    let creds = ilink::poll_qr_status(&qr.qrcode, |status| {
        if status != last_status {
            last_status = status.to_string();
            match status {
                "scaned" => println!("QR code scanned! Please confirm on your phone."),
                "confirmed" => println!("Login confirmed!"),
                "expired" => println!("QR code expired."),
                _ => {}
            }
        }
    })
    .await?;
    ilink::save_credentials(&creds)?;
    println!(
        "\nLogin successful! Credentials saved to {}\nBot ID: {}\n",
        ilink::credentials_path().display(),
        creds.ilink_bot_id
    );
    Ok(creds)
}

async fn run_send(to: String, text: String, media: String, account: String) -> Result<()> {
    if text.is_empty() && media.is_empty() {
        anyhow::bail!("at least one of --text or --media is required");
    }
    let accounts = ilink::load_all_credentials()?;
    let creds = select_account(&accounts, &account)
        .context("no matching account found, run `welinker accounts list`")?;
    let cfg = config::load().unwrap_or_default();
    let client = Client::new_with_route_tag(creds, Some(cfg.route_tag));
    if !text.is_empty() {
        send_text_reply(&client, &to, &text, "", None).await?;
        println!("Text sent");
    }
    if !media.is_empty() {
        send_media_from_url(&client, &to, &media, "").await?;
        println!("Media sent");
    }
    Ok(())
}

fn run_accounts(command: AccountCommands) -> Result<()> {
    match command {
        AccountCommands::List => {
            let accounts = ilink::load_all_credentials()?;
            if accounts.is_empty() {
                println!("No accounts found");
                return Ok(());
            }
            for account in accounts {
                println!(
                    "{}\tuser={}\tbase_url={}",
                    account.ilink_bot_id,
                    empty_dash(&account.ilink_user_id),
                    empty_dash(&account.baseurl)
                );
            }
            Ok(())
        }
        AccountCommands::Remove { account } => {
            let accounts = ilink::load_all_credentials()?;
            let creds = select_account(&accounts, &account)
                .context("no matching account found, run `welinker accounts list`")?;
            let path = ilink::accounts_dir().join(format!(
                "{}.json",
                ilink::normalize_account_id(&creds.ilink_bot_id)
            ));
            if path.exists() {
                fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
            }
            let sync_path = ilink::accounts_dir().join(format!(
                "{}.sync.json",
                ilink::normalize_account_id(&creds.ilink_bot_id)
            ));
            let _ = fs::remove_file(sync_path);
            println!("Removed account {}", creds.ilink_bot_id);
            Ok(())
        }
    }
}

fn select_account<'a>(
    accounts: &'a [ilink::Credentials],
    requested: &str,
) -> Option<&'a ilink::Credentials> {
    if requested.is_empty() {
        return accounts.first();
    }
    accounts.iter().find(|account| {
        account.ilink_bot_id == requested
            || ilink::normalize_account_id(&account.ilink_bot_id) == requested
            || account.ilink_user_id == requested
    })
}

fn empty_dash(value: &str) -> &str {
    if value.is_empty() {
        "-"
    } else {
        value
    }
}

fn init_logging(to_file: bool) -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if to_file {
        fs::create_dir_all(app_dir())?;
        let file_appender = tracing_appender::rolling::never(app_dir(), "welinker.log");
        fmt()
            .with_env_filter(filter)
            .with_writer(file_appender)
            .try_init()
            .ok();
    } else {
        fmt().with_env_filter(filter).try_init().ok();
    }
    Ok(())
}

fn app_dir() -> PathBuf {
    config::app_dir()
}

fn pid_file() -> PathBuf {
    app_dir().join("welinker.pid")
}

fn log_file() -> PathBuf {
    app_dir().join("welinker.log")
}

fn run_daemon(web_only: bool) -> Result<()> {
    stop_all();
    fs::create_dir_all(app_dir())?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file())?;
    let err = log.try_clone()?;
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.args(["start", "--foreground"]);
    if web_only {
        cmd.arg("--web-only");
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                nix::unistd::setsid().map_err(std::io::Error::other)?;
                Ok(())
            });
        }
    }
    let child = cmd.spawn().context("start daemon")?;
    let pid = child.id();
    fs::write(pid_file(), pid.to_string())?;
    println!("welinker started in background (pid={pid})");
    println!("Log: {}", log_file().display());
    println!("Stop: welinker stop");
    Ok(())
}

fn run_status() {
    match read_pid() {
        Some(pid) if process_exists(pid) => {
            println!("welinker is running (pid={pid})");
            println!("Log: {}", log_file().display());
        }
        Some(_) => println!("welinker is not running (stale pid file)"),
        None => println!("welinker is not running"),
    }
}

fn stop_all() {
    if let Some(pid) = read_pid() {
        let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
    }
    let _ = fs::remove_file(pid_file());
}

fn read_pid() -> Option<u32> {
    fs::read_to_string(pid_file()).ok()?.trim().parse().ok()
}

fn process_exists(pid: u32) -> bool {
    kill(Pid::from_raw(pid as i32), None).is_ok()
}
