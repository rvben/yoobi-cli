//! Agent-friendly command interface: text on a terminal, JSON when piped.
use clap::error::ErrorKind as ClapErrorKind;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use std::io::{IsTerminal, Write};
use std::process::ExitCode;
use yoobi_cli::{
    Error,
    api::Client,
    booking::{BookingEdit, parse_duration},
    config::Config,
    schema,
    timesheet::validate_month,
};

#[derive(Parser)]
#[command(
    name = "yoobi-cli",
    version,
    about = "Read and browse Yoobi timesheets",
    long_about = "Read and browse Yoobi timesheets.\n\nRun `yoobi-cli schema` for the machine-readable contract."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    /// Output format; auto = text on a terminal, JSON when piped.
    #[arg(long, short = 'o', value_enum, default_value = "auto", global = true)]
    output: Output,
}
#[derive(Clone, Copy, ValueEnum)]
enum Output {
    Auto,
    Json,
    Text,
}
#[derive(Subcommand)]
enum Command {
    /// Inspect a month, including each project's booked minutes and locked dates.
    Timesheet {
        #[arg(long)]
        month: String,
    },
    /// List the booking activities available in a month.
    Projects {
        #[arg(long)]
        month: String,
    },
    /// Preview or save an exact booking duration.
    Booking {
        #[command(subcommand)]
        command: BookingCommand,
    },
    /// Save, inspect or remove a login in the OS credential store.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Print the machine-readable clispec contract.
    Schema,
    /// Browse booked hours and explicitly review and save exact booking edits.
    Tui {
        #[arg(long)]
        month: Option<String>,
        #[arg(long)]
        demo: bool,
        #[arg(long, requires = "demo")]
        snapshot: bool,
        #[arg(long, default_value_t = 120, requires = "snapshot")]
        width: u16,
        #[arg(long, default_value_t = 36, requires = "snapshot")]
        height: u16,
    },
    /// Generate shell completions.
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}
#[derive(Subcommand)]
enum BookingCommand {
    /// Read the current booking and preview an exact new total; does not save.
    Preview {
        #[arg(long)]
        activity: String,
        #[arg(long)]
        date: String,
        /// Exact total, e.g. 7h30, 7:30 or 450m. Use 0m to clear.
        #[arg(long)]
        duration: String,
    },
    /// Save a reviewed JSON preview, then read back to verify.
    Save {
        file: std::path::PathBuf,
        /// Explicitly authorize the preview's booking change.
        #[arg(long, required = true)]
        yes: bool,
    },
}
#[derive(Subcommand)]
enum AuthCommand {
    /// Verify credentials and save a reusable login in the OS credential store.
    Login {
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        username: Option<String>,
        /// Read password from stdin instead of prompting (never pass it as an argument).
        #[arg(long)]
        password_stdin: bool,
    },
    /// Forget the saved login. Explicit environment credentials remain unaffected.
    Logout,
    Status,
}

fn client() -> Result<Client, Error> {
    Client::new(&Config::load()?)
}
fn dispatch(command: Command) -> Result<Value, Error> {
    match command {
        Command::Timesheet { month } => {
            validate_month(&month)?;
            Ok(json!(client()?.timesheet(&month)?))
        }
        Command::Projects { month } => {
            validate_month(&month)?;
            let sheet = client()?.timesheet(&month)?;
            Ok(
                json!({"month":sheet.month,"employee_id":sheet.employee_id,"projects":sheet.projects.iter().map(|p|json!({"id":p.id,"name":p.name})).collect::<Vec<_>>()}),
            )
        }
        Command::Booking {
            command:
                BookingCommand::Preview {
                    activity,
                    date,
                    duration,
                },
        } => {
            yoobi_cli::timesheet::validate_date(&date)?;
            let minutes = parse_duration(&duration)?;
            if activity.trim().is_empty() {
                return Err(Error::InvalidInput {
                    message: "activity is required".into(),
                });
            }
            Ok(json!(client()?.preview_booking(&activity, &date, minutes)?))
        }
        Command::Booking {
            command: BookingCommand::Save { file, yes: _ },
        } => {
            use std::io::Read;
            let file = std::fs::File::open(file).map_err(|_| Error::Io {
                message: "cannot open booking preview".into(),
            })?;
            let mut data = String::new();
            file.take(65537)
                .read_to_string(&mut data)
                .map_err(|_| Error::InvalidInput {
                    message: "cannot read booking preview".into(),
                })?;
            if data.len() > 65536 {
                return Err(Error::InvalidInput {
                    message: "booking preview exceeds 64 KiB".into(),
                });
            }
            let edit: BookingEdit =
                serde_json::from_str(&data).map_err(|_| Error::InvalidInput {
                    message: "invalid booking preview JSON".into(),
                })?;
            edit.validate()?;
            Ok(json!(client()?.save_booking(&edit)?))
        }
        Command::Auth {
            command:
                AuthCommand::Login {
                    base_url,
                    username,
                    password_stdin,
                },
        } => yoobi_cli::auth::login(base_url, username, password_stdin),
        Command::Auth {
            command: AuthCommand::Logout,
        } => yoobi_cli::auth::forget(),
        Command::Auth {
            command: AuthCommand::Status,
        } => Ok(Config::status()),
        Command::Tui { .. } | Command::Schema | Command::Completions { .. } => {
            unreachable!("handled before dispatch")
        }
    }
}
fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => return clap_error(e),
    };
    if let Command::Tui {
        month,
        demo,
        snapshot,
        width,
        height,
    } = &cli.command
    {
        let result = (|| {
            // Refuse piped interactive input before credentials are read.
            if !snapshot && (!std::io::stdin().is_terminal() || !std::io::stdout().is_terminal()) {
                return Err(Error::TtyRequired {
                    message: "tui requires terminal stdin and stdout".into(),
                });
            }
            let options = yoobi_cli::tui::Options {
                month: month.clone().unwrap_or_else(yoobi_cli::tui::current_month),
                demo: *demo,
                snapshot: *snapshot,
                width: *width,
                height: *height,
            };
            yoobi_cli::tui::validate(&options, matches!(cli.output, Output::Json))?;
            if *snapshot {
                println!("{}", yoobi_cli::tui::snapshot(&options)?);
                Ok(())
            } else {
                yoobi_cli::tui::run(options)
            }
        })();
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => emit_error(e),
        };
    }
    match &cli.command {
        Command::Schema => {
            println!("{}", schema::contract_json());
            return ExitCode::SUCCESS;
        }
        Command::Completions { shell } => {
            clap_complete::generate(
                *shell,
                &mut Cli::command(),
                "yoobi-cli",
                &mut std::io::stdout(),
            );
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    match dispatch(cli.command) {
        Ok(value) => {
            let text = matches!(cli.output, Output::Text)
                || matches!(cli.output, Output::Auto) && std::io::stdout().is_terminal();
            let rendered = if text {
                yoobi_cli::output::text(&value)
            } else {
                value.to_string()
            };
            if writeln!(std::io::stdout(), "{rendered}").is_err() {
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        Err(err) => emit_error(err),
    }
}
fn clap_error(e: clap::Error) -> ExitCode {
    match e.kind() {
        ClapErrorKind::DisplayHelp | ClapErrorKind::DisplayVersion => {
            let _ = e.print();
            ExitCode::SUCCESS
        }
        _ => emit_error(Error::Usage {
            message: e.to_string().trim().into(),
        }),
    }
}
fn emit_error(err: Error) -> ExitCode {
    let mut error =
        json!({"kind":err.kind(),"message":err.to_string(),"exit_code":err.exit_code()});
    if let Some(hint) = err.hint() {
        error["hint"] = json!(hint);
    }
    eprintln!("{}", json!({"error":error}));
    ExitCode::from(err.exit_code() as u8)
}
