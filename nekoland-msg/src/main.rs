//! CLI front-end for the compositor IPC socket.
//!
//! Argument parsing normalizes modern subcommands, hidden compatibility aliases, completion
//! generation, and long-running subscription mode into a single `ParsedAction`.

use std::ffi::OsString;
use std::process::ExitCode;

use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand, ValueEnum, error::ErrorKind};
use clap_complete::{Shell, generate};
use nekoland_ipc::commands::{
    ActionCommand, OutputCommand, PopupCommand, QueryCommand, SplitAxis, WindowCommand,
    WorkspaceCommand,
};
use nekoland_ipc::{
    IpcCommand, IpcRequest, IpcSubscription, KNOWN_SUBSCRIPTION_EVENT_NAMES,
    SUPPORTED_SUBSCRIPTION_TOPIC_NAMES, SubscriptionTopic, default_socket_path, send_request,
    subscribe,
};

const USAGE: &str = "usage:
  nekoland-msg <query|window|popup|workspace|output|action> ...
  nekoland-msg subscribe <window|popup|workspace|output|command|config|keyboard-layout|clipboard|primary-selection|focus|tree|all> [--pretty|--jsonl] [--no-payloads] [--event <name|prefix*>]...";
const SUBSCRIPTION_HELP_EXAMPLES: &[&str] = &[
    "nekoland-msg subscribe workspace",
    "nekoland-msg subscribe command --event command_*",
    "nekoland-msg subscribe config --event config_changed",
    "nekoland-msg subscribe focus --event focus_changed",
    "nekoland-msg subscribe all --event window_* --event tree_* --jsonl --no-payloads",
];

/// Top-level CLI parser.
#[derive(Parser, Debug)]
#[command(name = "nekoland-msg", disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: RootCommand,
}

/// Top-level subcommands and compatibility aliases supported by the CLI.
#[derive(Subcommand, Debug)]
enum RootCommand {
    Query(QueryArgs),
    Window(WindowArgs),
    Popup(PopupArgs),
    Workspace(WorkspaceArgs),
    Output(OutputArgs),
    Action(ActionArgs),
    Completion(CompletionArgs),
    Subscribe(SubscribeArgs),
    Help(HelpArgs),
    #[command(name = "get_tree", hide = true)]
    GetTree,
    #[command(name = "get_outputs", hide = true)]
    GetOutputs,
    #[command(name = "get_workspaces", hide = true)]
    GetWorkspaces,
    #[command(name = "get_windows", hide = true)]
    GetWindows,
    #[command(name = "get_commands", hide = true)]
    GetCommands,
    #[command(name = "get_config", hide = true)]
    GetConfig,
    #[command(name = "get_keyboard_layouts", hide = true)]
    GetKeyboardLayouts,
    #[command(name = "get_clipboard", hide = true)]
    GetClipboard,
    #[command(name = "get_primary_selection", hide = true)]
    GetPrimarySelection,
    #[command(external_subcommand)]
    Raw(Vec<String>),
}

/// Arguments for the `query` subcommand family.
#[derive(Args, Debug)]
struct QueryArgs {
    #[command(subcommand)]
    target: QueryTarget,
}

/// Read-only query targets supported by the CLI.
#[derive(Subcommand, Debug)]
enum QueryTarget {
    Tree,
    Outputs,
    Workspaces,
    Windows,
    KeyboardLayouts,
    Commands,
    Config,
    Clipboard,
    PrimarySelection,
}

/// Arguments for the `window` subcommand family.
#[derive(Args, Debug)]
struct WindowArgs {
    #[command(subcommand)]
    action: WindowAction,
}

/// Window-management actions supported by the CLI.
#[derive(Subcommand, Debug)]
enum WindowAction {
    Focus {
        surface_id: u64,
    },
    Close {
        surface_id: u64,
    },
    Move {
        surface_id: u64,
        #[arg(allow_hyphen_values = true)]
        x: i64,
        #[arg(allow_hyphen_values = true)]
        y: i64,
    },
    Resize {
        surface_id: u64,
        width: u32,
        height: u32,
    },
    Split {
        surface_id: u64,
        axis: SplitAxisArg,
    },
    Background {
        surface_id: u64,
        output: String,
    },
    ClearBackground {
        surface_id: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SplitAxisArg {
    Horizontal,
    Vertical,
}

impl From<SplitAxisArg> for SplitAxis {
    fn from(value: SplitAxisArg) -> Self {
        match value {
            SplitAxisArg::Horizontal => Self::Horizontal,
            SplitAxisArg::Vertical => Self::Vertical,
        }
    }
}

/// Arguments for the `popup` subcommand family.
#[derive(Args, Debug)]
struct PopupArgs {
    #[command(subcommand)]
    action: PopupAction,
}

/// Popup-management actions supported by the CLI.
#[derive(Subcommand, Debug)]
enum PopupAction {
    Dismiss { surface_id: u64 },
}

/// Arguments for the `workspace` subcommand family.
#[derive(Args, Debug)]
struct WorkspaceArgs {
    #[command(subcommand)]
    action: WorkspaceAction,
}

/// Workspace-management actions supported by the CLI.
#[derive(Subcommand, Debug)]
enum WorkspaceAction {
    Switch { workspace: String },
    Create { workspace: String },
    Destroy { workspace: String },
}

/// Arguments for the `output` subcommand family.
#[derive(Args, Debug)]
struct OutputArgs {
    #[command(subcommand)]
    action: OutputAction,
}

/// Output-management actions supported by the CLI.
#[derive(Subcommand, Debug)]
enum OutputAction {
    Enable {
        output: String,
    },
    Disable {
        output: String,
    },
    Configure {
        output: String,
        mode: String,
        scale: Option<u32>,
    },
    ViewportMove {
        output: String,
        #[arg(allow_hyphen_values = true)]
        x: i64,
        #[arg(allow_hyphen_values = true)]
        y: i64,
    },
    ViewportPan {
        output: String,
        #[arg(allow_hyphen_values = true)]
        dx: i64,
        #[arg(allow_hyphen_values = true)]
        dy: i64,
    },
    CenterViewportOnWindow {
        output: String,
        surface_id: u64,
    },
}

/// Arguments for the higher-level `action` command family.
#[derive(Args, Debug)]
struct ActionArgs {
    #[command(subcommand)]
    action: ActionAction,
}

/// Shell-style higher-level actions exposed for external shell integration.
#[derive(Subcommand, Debug)]
enum ActionAction {
    FocusWorkspace {
        workspace: String,
    },
    FocusWindow {
        #[arg(long = "id")]
        id: u64,
    },
    CloseWindow {
        #[arg(long = "id")]
        id: u64,
    },
    Spawn {
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    SwitchKeyboardLayoutNext,
    SwitchKeyboardLayoutPrev,
    SwitchKeyboardLayoutName {
        name: String,
    },
    SwitchKeyboardLayoutIndex {
        index: usize,
    },
    ReloadConfig,
    Quit,
    PowerOffMonitors,
    PowerOnMonitors,
}

/// Arguments for shell-completion generation.
#[derive(Args, Debug)]
struct CompletionArgs {
    shell: CompletionShellArg,
}

/// Rendering mode for subscription output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubscriptionOutputMode {
    Pretty,
    Jsonl,
}

/// Output mode for CLI help rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelpOutputMode {
    Text,
    Json,
}

/// Arguments for the `subscribe` command.
#[derive(Args, Debug)]
#[command(disable_help_flag = true)]
struct SubscribeArgs {
    topic: Option<SubscriptionTopicArg>,
    #[arg(short = 'h', long = "help", action = ArgAction::SetTrue)]
    help: bool,
    #[arg(long, action = ArgAction::SetTrue)]
    json: bool,
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "jsonl")]
    pretty: bool,
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "pretty")]
    jsonl: bool,
    #[arg(long = "no-payloads", action = ArgAction::SetTrue)]
    no_payloads: bool,
    #[arg(long = "event", action = ArgAction::Append)]
    events: Vec<String>,
}

/// Arguments for specialized help topics.
#[derive(Args, Debug)]
struct HelpArgs {
    #[command(subcommand)]
    topic: HelpTopic,
}

/// Help topics exposed as first-class subcommands.
#[derive(Subcommand, Debug)]
enum HelpTopic {
    Subscribe(SubscribeHelpArgs),
}

/// Arguments for `help subscribe`.
#[derive(Args, Debug, Default)]
struct SubscribeHelpArgs {
    #[arg(long, action = ArgAction::SetTrue)]
    json: bool,
}

/// User-facing topic names accepted by the subscribe CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SubscriptionTopicArg {
    Window,
    Popup,
    Workspace,
    Output,
    Command,
    Config,
    KeyboardLayout,
    Clipboard,
    PrimarySelection,
    Focus,
    Tree,
    All,
}

/// Shells supported by completion generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CompletionShellArg {
    Bash,
    Zsh,
    Fish,
}

/// Normalized subscribe command after parsing CLI flags.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SubscriptionCommand {
    subscription: IpcSubscription,
    output_mode: SubscriptionOutputMode,
}

/// Unified execution plan returned by CLI parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedAction {
    Request(IpcCommand),
    Completion(CompletionShellArg),
    Subscribe(SubscriptionCommand),
    SubscriptionHelp(HelpOutputMode),
}

fn main() -> ExitCode {
    let action = match parse_cli_from(std::env::args_os()) {
        Ok(action) => action,
        Err(error) => {
            let _ = error.print();
            return if error.use_stderr() { ExitCode::FAILURE } else { ExitCode::SUCCESS };
        }
    };

    match action {
        ParsedAction::SubscriptionHelp(mode) => match render_subscription_help(mode) {
            Ok(help) => {
                println!("{help}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("failed to encode subscription help: {error}");
                ExitCode::FAILURE
            }
        },
        ParsedAction::Completion(shell) => match render_completion(shell) {
            Ok(script) => {
                print!("{script}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("failed to render shell completion: {error}");
                ExitCode::FAILURE
            }
        },
        ParsedAction::Subscribe(command) => run_subscription(command),
        ParsedAction::Request(command) => send_ipc_command(command),
    }
}

/// Collapses clap's parse tree into a single execution enum so the runtime path below can stay
/// small even though the CLI still exposes aliases and specialized help/completion flows.
fn parse_cli_from<I, T>(args: I) -> Result<ParsedAction, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)?;
    match cli.command {
        RootCommand::Query(query) => Ok(ParsedAction::Request(match query.target {
            QueryTarget::Tree => IpcCommand::Query(QueryCommand::GetTree),
            QueryTarget::Outputs => IpcCommand::Query(QueryCommand::GetOutputs),
            QueryTarget::Workspaces => IpcCommand::Query(QueryCommand::GetWorkspaces),
            QueryTarget::Windows => IpcCommand::Query(QueryCommand::GetWindows),
            QueryTarget::KeyboardLayouts => IpcCommand::Query(QueryCommand::GetKeyboardLayouts),
            QueryTarget::Commands => IpcCommand::Query(QueryCommand::GetCommands),
            QueryTarget::Config => IpcCommand::Query(QueryCommand::GetConfig),
            QueryTarget::Clipboard => IpcCommand::Query(QueryCommand::GetClipboard),
            QueryTarget::PrimarySelection => IpcCommand::Query(QueryCommand::GetPrimarySelection),
        })),
        RootCommand::Window(window) => Ok(ParsedAction::Request(match window.action {
            WindowAction::Focus { surface_id } => {
                IpcCommand::Window(WindowCommand::Focus { surface_id })
            }
            WindowAction::Close { surface_id } => {
                IpcCommand::Window(WindowCommand::Close { surface_id })
            }
            WindowAction::Move { surface_id, x, y } => {
                IpcCommand::Window(WindowCommand::Move { surface_id, x, y })
            }
            WindowAction::Resize { surface_id, width, height } => {
                IpcCommand::Window(WindowCommand::Resize { surface_id, width, height })
            }
            WindowAction::Split { surface_id, axis } => {
                IpcCommand::Window(WindowCommand::Split { surface_id, axis: axis.into() })
            }
            WindowAction::Background { surface_id, output } => {
                IpcCommand::Window(WindowCommand::Background { surface_id, output })
            }
            WindowAction::ClearBackground { surface_id } => {
                IpcCommand::Window(WindowCommand::ClearBackground { surface_id })
            }
        })),
        RootCommand::Popup(popup) => Ok(ParsedAction::Request(match popup.action {
            PopupAction::Dismiss { surface_id } => {
                IpcCommand::Popup(PopupCommand::Dismiss { surface_id })
            }
        })),
        RootCommand::Workspace(workspace) => Ok(ParsedAction::Request(match workspace.action {
            WorkspaceAction::Switch { workspace } => {
                IpcCommand::Workspace(WorkspaceCommand::Switch { workspace })
            }
            WorkspaceAction::Create { workspace } => {
                IpcCommand::Workspace(WorkspaceCommand::Create { workspace })
            }
            WorkspaceAction::Destroy { workspace } => {
                IpcCommand::Workspace(WorkspaceCommand::Destroy { workspace })
            }
        })),
        RootCommand::Output(output) => Ok(ParsedAction::Request(match output.action {
            OutputAction::Enable { output } => IpcCommand::Output(OutputCommand::Enable { output }),
            OutputAction::Disable { output } => {
                IpcCommand::Output(OutputCommand::Disable { output })
            }
            OutputAction::Configure { output, mode, scale } => {
                IpcCommand::Output(OutputCommand::Configure { output, mode, scale })
            }
            OutputAction::ViewportMove { output, x, y } => {
                IpcCommand::Output(OutputCommand::ViewportMove { output, x, y })
            }
            OutputAction::ViewportPan { output, dx, dy } => {
                IpcCommand::Output(OutputCommand::ViewportPan { output, dx, dy })
            }
            OutputAction::CenterViewportOnWindow { output, surface_id } => {
                IpcCommand::Output(OutputCommand::CenterViewportOnWindow { output, surface_id })
            }
        })),
        RootCommand::Action(action) => Ok(ParsedAction::Request(match action.action {
            ActionAction::FocusWorkspace { workspace } => {
                IpcCommand::Action(ActionCommand::FocusWorkspace { workspace })
            }
            ActionAction::FocusWindow { id } => {
                IpcCommand::Action(ActionCommand::FocusWindow { id })
            }
            ActionAction::CloseWindow { id } => {
                IpcCommand::Action(ActionCommand::CloseWindow { id })
            }
            ActionAction::Spawn { command } => IpcCommand::Action(ActionCommand::Spawn { command }),
            ActionAction::SwitchKeyboardLayoutNext => {
                IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutNext)
            }
            ActionAction::SwitchKeyboardLayoutPrev => {
                IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutPrev)
            }
            ActionAction::SwitchKeyboardLayoutName { name } => {
                IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutByName { name })
            }
            ActionAction::SwitchKeyboardLayoutIndex { index } => {
                IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutByIndex { index })
            }
            ActionAction::ReloadConfig => IpcCommand::Action(ActionCommand::ReloadConfig),
            ActionAction::Quit => IpcCommand::Action(ActionCommand::Quit),
            ActionAction::PowerOffMonitors => IpcCommand::Action(ActionCommand::PowerOffMonitors),
            ActionAction::PowerOnMonitors => IpcCommand::Action(ActionCommand::PowerOnMonitors),
        })),
        RootCommand::Completion(completion) => Ok(ParsedAction::Completion(completion.shell)),
        RootCommand::Subscribe(subscribe) => parse_subscribe_args(subscribe),
        RootCommand::Help(help) => Ok(ParsedAction::SubscriptionHelp(match help.topic {
            HelpTopic::Subscribe(topic) => {
                if topic.json {
                    HelpOutputMode::Json
                } else {
                    HelpOutputMode::Text
                }
            }
        })),
        RootCommand::GetTree => Ok(ParsedAction::Request(IpcCommand::Query(QueryCommand::GetTree))),
        RootCommand::GetOutputs => {
            Ok(ParsedAction::Request(IpcCommand::Query(QueryCommand::GetOutputs)))
        }
        RootCommand::GetWorkspaces => {
            Ok(ParsedAction::Request(IpcCommand::Query(QueryCommand::GetWorkspaces)))
        }
        RootCommand::GetWindows => {
            Ok(ParsedAction::Request(IpcCommand::Query(QueryCommand::GetWindows)))
        }
        RootCommand::GetCommands => {
            Ok(ParsedAction::Request(IpcCommand::Query(QueryCommand::GetCommands)))
        }
        RootCommand::GetConfig => {
            Ok(ParsedAction::Request(IpcCommand::Query(QueryCommand::GetConfig)))
        }
        RootCommand::GetKeyboardLayouts => {
            Ok(ParsedAction::Request(IpcCommand::Query(QueryCommand::GetKeyboardLayouts)))
        }
        RootCommand::GetClipboard => {
            Ok(ParsedAction::Request(IpcCommand::Query(QueryCommand::GetClipboard)))
        }
        RootCommand::GetPrimarySelection => {
            Ok(ParsedAction::Request(IpcCommand::Query(QueryCommand::GetPrimarySelection)))
        }
        RootCommand::Raw(raw) => Ok(ParsedAction::Request(IpcCommand::Raw(raw.join(" ")))),
    }
}

fn parse_subscribe_args(args: SubscribeArgs) -> Result<ParsedAction, clap::Error> {
    if args.help {
        return Ok(ParsedAction::SubscriptionHelp(if args.json {
            HelpOutputMode::Json
        } else {
            HelpOutputMode::Text
        }));
    }

    if args.json {
        return Err(clap::Error::raw(
            ErrorKind::UnknownArgument,
            "`--json` is only supported with `subscribe --help` or `help subscribe`",
        ));
    }

    let Some(topic) = args.topic else {
        return Err(clap::Error::raw(ErrorKind::MissingRequiredArgument, subscription_help_text()));
    };

    let _ = args.pretty;

    Ok(ParsedAction::Subscribe(SubscriptionCommand {
        subscription: IpcSubscription {
            topic: topic.into(),
            include_payloads: !args.no_payloads,
            events: dedupe_events(args.events),
        },
        output_mode: if args.jsonl {
            SubscriptionOutputMode::Jsonl
        } else {
            SubscriptionOutputMode::Pretty
        },
    }))
}

/// Removes duplicate `--event` filters while preserving the user's original order.
fn dedupe_events(events: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::with_capacity(events.len());
    for event in events {
        if !deduped.iter().any(|existing| existing == &event) {
            deduped.push(event);
        }
    }
    deduped
}

impl From<SubscriptionTopicArg> for SubscriptionTopic {
    /// Converts CLI-facing topic names into the shared IPC subscription topic enum.
    fn from(value: SubscriptionTopicArg) -> Self {
        match value {
            SubscriptionTopicArg::Window => SubscriptionTopic::Window,
            SubscriptionTopicArg::Popup => SubscriptionTopic::Popup,
            SubscriptionTopicArg::Workspace => SubscriptionTopic::Workspace,
            SubscriptionTopicArg::Output => SubscriptionTopic::Output,
            SubscriptionTopicArg::Command => SubscriptionTopic::Command,
            SubscriptionTopicArg::Config => SubscriptionTopic::Config,
            SubscriptionTopicArg::KeyboardLayout => SubscriptionTopic::KeyboardLayout,
            SubscriptionTopicArg::Clipboard => SubscriptionTopic::Clipboard,
            SubscriptionTopicArg::PrimarySelection => SubscriptionTopic::PrimarySelection,
            SubscriptionTopicArg::Focus => SubscriptionTopic::Focus,
            SubscriptionTopicArg::Tree => SubscriptionTopic::Tree,
            SubscriptionTopicArg::All => SubscriptionTopic::All,
        }
    }
}

impl From<CompletionShellArg> for Shell {
    /// Converts CLI-facing shell names into clap-complete's shell enum.
    fn from(value: CompletionShellArg) -> Self {
        match value {
            CompletionShellArg::Bash => Shell::Bash,
            CompletionShellArg::Zsh => Shell::Zsh,
            CompletionShellArg::Fish => Shell::Fish,
        }
    }
}

/// Renders subscription help either as plain text or as machine-readable JSON.
fn render_subscription_help(mode: HelpOutputMode) -> serde_json::Result<String> {
    match mode {
        HelpOutputMode::Text => Ok(subscription_help_text()),
        HelpOutputMode::Json => subscription_help_json(),
    }
}

/// Generates shell completion output for the requested shell.
fn render_completion(shell: CompletionShellArg) -> Result<String, std::string::FromUtf8Error> {
    let mut command = Cli::command();
    let mut output = Vec::new();
    let shell: Shell = shell.into();
    generate(shell, &mut command, "nekoland-msg", &mut output);
    String::from_utf8(output)
}

/// Builds the human-readable help text shown by `subscribe --help`.
fn subscription_help_text() -> String {
    format!(
        "{USAGE}\n\nTopics:\n  {}\n\nKnown events:\n  {}\n\nPatterns:\n  exact match: tree_changed\n  prefix wildcard: window_*\n\nExamples:\n  {}\n  {}",
        SUPPORTED_SUBSCRIPTION_TOPIC_NAMES.join("\n  "),
        KNOWN_SUBSCRIPTION_EVENT_NAMES.join("\n  "),
        SUBSCRIPTION_HELP_EXAMPLES[0],
        SUBSCRIPTION_HELP_EXAMPLES[1],
    )
}

/// Builds the machine-readable JSON payload returned by `subscribe --help --json`.
fn subscription_help_json() -> serde_json::Result<String> {
    serde_json::to_string_pretty(&serde_json::json!({
        "usage": USAGE,
        "topics": SUPPORTED_SUBSCRIPTION_TOPIC_NAMES,
        "known_events": KNOWN_SUBSCRIPTION_EVENT_NAMES,
        "patterns": {
            "exact_match_example": "tree_changed",
            "prefix_wildcard_example": "window_*",
        },
        "examples": SUBSCRIPTION_HELP_EXAMPLES,
    }))
}

/// Runs the long-lived subscription loop until the server disconnects or an unrecoverable error
/// occurs.
fn run_subscription(command: SubscriptionCommand) -> ExitCode {
    let mut stream = match subscribe(&command.subscription) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("IPC subscription failed: {error}");
            return ExitCode::FAILURE;
        }
    };

    loop {
        match stream.read_event() {
            Ok(event) => match format_subscription_event(&event, command.output_mode) {
                Ok(message) => println!("{message}"),
                Err(error) => {
                    eprintln!("failed to encode subscription event: {error}");
                    return ExitCode::FAILURE;
                }
            },
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if !default_socket_path().exists() {
                    return ExitCode::SUCCESS;
                }
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                return ExitCode::SUCCESS;
            }
            Err(error) => {
                eprintln!("IPC subscription read failed: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
}

/// Formats one subscription event as either pretty JSON or one-line JSONL.
fn format_subscription_event(
    event: &nekoland_ipc::IpcSubscriptionEvent,
    output_mode: SubscriptionOutputMode,
) -> serde_json::Result<String> {
    match output_mode {
        SubscriptionOutputMode::Pretty => serde_json::to_string_pretty(event),
        SubscriptionOutputMode::Jsonl => serde_json::to_string(event),
    }
}

/// Sends one request/response IPC command and prints either the payload body or the full reply.
fn send_ipc_command(command: IpcCommand) -> ExitCode {
    let request = IpcRequest { correlation_id: 1, command };

    match send_request(&request) {
        Ok(reply) => {
            if let Some(payload) = &reply.payload {
                match serde_json::to_string_pretty(payload) {
                    Ok(message) => println!("{message}"),
                    Err(error) => {
                        eprintln!("failed to encode IPC payload: {error}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                match serde_json::to_string_pretty(&reply) {
                    Ok(message) => println!("{message}"),
                    Err(error) => {
                        eprintln!("failed to encode IPC reply: {error}");
                        return ExitCode::FAILURE;
                    }
                }
            }

            if reply.ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        }
        Err(error) => {
            eprintln!("IPC request failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CompletionShellArg, HelpOutputMode, ParsedAction, SplitAxis, SubscriptionCommand,
        SubscriptionOutputMode, parse_cli_from, render_completion, render_subscription_help,
    };
    use nekoland_ipc::commands::{
        ActionCommand, OutputCommand, PopupCommand, QueryCommand, WindowCommand,
    };
    use nekoland_ipc::{IpcCommand, IpcSubscription, SubscriptionTopic};
    use serde_json::Value;

    fn parse_ok(args: impl IntoIterator<Item = &'static str>) -> ParsedAction {
        let Ok(action) = parse_cli_from(args) else {
            panic!("CLI arguments should parse successfully");
        };
        action
    }

    #[test]
    fn parses_query_tree_alias() {
        assert_eq!(
            parse_ok(["nekoland-msg", "query", "tree"]),
            ParsedAction::Request(IpcCommand::Query(QueryCommand::GetTree))
        );
        assert_eq!(
            parse_ok(["nekoland-msg", "get_tree"]),
            ParsedAction::Request(IpcCommand::Query(QueryCommand::GetTree))
        );
    }

    #[test]
    fn parses_query_commands_alias() {
        assert_eq!(
            parse_ok(["nekoland-msg", "query", "commands"]),
            ParsedAction::Request(IpcCommand::Query(QueryCommand::GetCommands))
        );
        assert_eq!(
            parse_ok(["nekoland-msg", "get_commands"]),
            ParsedAction::Request(IpcCommand::Query(QueryCommand::GetCommands))
        );
    }

    #[test]
    fn parses_query_windows_alias() {
        assert_eq!(
            parse_ok(["nekoland-msg", "query", "windows"]),
            ParsedAction::Request(IpcCommand::Query(QueryCommand::GetWindows))
        );
        assert_eq!(
            parse_ok(["nekoland-msg", "get_windows"]),
            ParsedAction::Request(IpcCommand::Query(QueryCommand::GetWindows))
        );
    }

    #[test]
    fn parses_query_keyboard_layouts_alias() {
        assert_eq!(
            parse_ok(["nekoland-msg", "query", "keyboard-layouts"]),
            ParsedAction::Request(IpcCommand::Query(QueryCommand::GetKeyboardLayouts))
        );
        assert_eq!(
            parse_ok(["nekoland-msg", "get_keyboard_layouts"]),
            ParsedAction::Request(IpcCommand::Query(QueryCommand::GetKeyboardLayouts))
        );
    }

    #[test]
    fn parses_query_config_alias() {
        assert_eq!(
            parse_ok(["nekoland-msg", "query", "config"]),
            ParsedAction::Request(IpcCommand::Query(QueryCommand::GetConfig))
        );
        assert_eq!(
            parse_ok(["nekoland-msg", "get_config"]),
            ParsedAction::Request(IpcCommand::Query(QueryCommand::GetConfig))
        );
    }

    #[test]
    fn parses_query_clipboard_alias() {
        assert_eq!(
            parse_ok(["nekoland-msg", "query", "clipboard"]),
            ParsedAction::Request(IpcCommand::Query(QueryCommand::GetClipboard))
        );
        assert_eq!(
            parse_ok(["nekoland-msg", "get_clipboard"]),
            ParsedAction::Request(IpcCommand::Query(QueryCommand::GetClipboard))
        );
    }

    #[test]
    fn parses_query_primary_selection_alias() {
        assert_eq!(
            parse_ok(["nekoland-msg", "query", "primary-selection"]),
            ParsedAction::Request(IpcCommand::Query(QueryCommand::GetPrimarySelection))
        );
        assert_eq!(
            parse_ok(["nekoland-msg", "get_primary_selection"]),
            ParsedAction::Request(IpcCommand::Query(QueryCommand::GetPrimarySelection))
        );
    }

    #[test]
    fn parses_window_move() {
        assert_eq!(
            parse_ok(["nekoland-msg", "window", "move", "7", "12", "-4"]),
            ParsedAction::Request(IpcCommand::Window(WindowCommand::Move {
                surface_id: 7,
                x: 12,
                y: -4
            }))
        );
    }

    #[test]
    fn parses_window_background() {
        assert_eq!(
            parse_ok(["nekoland-msg", "window", "background", "7", "Virtual-1"]),
            ParsedAction::Request(IpcCommand::Window(WindowCommand::Background {
                surface_id: 7,
                output: "Virtual-1".to_owned(),
            }))
        );
    }

    #[test]
    fn parses_output_viewport_pan() {
        assert_eq!(
            parse_ok(["nekoland-msg", "output", "viewport-pan", "Virtual-1", "-40", "25"]),
            ParsedAction::Request(IpcCommand::Output(OutputCommand::ViewportPan {
                output: "Virtual-1".to_owned(),
                dx: -40,
                dy: 25,
            }))
        );
    }

    #[test]
    fn parses_output_center_viewport_on_window() {
        assert_eq!(
            parse_ok(["nekoland-msg", "output", "center-viewport-on-window", "Virtual-1", "77"]),
            ParsedAction::Request(IpcCommand::Output(OutputCommand::CenterViewportOnWindow {
                output: "Virtual-1".to_owned(),
                surface_id: 77,
            }))
        );
    }

    #[test]
    fn parses_window_split() {
        assert_eq!(
            parse_ok(["nekoland-msg", "window", "split", "7", "vertical"]),
            ParsedAction::Request(IpcCommand::Window(WindowCommand::Split {
                surface_id: 7,
                axis: SplitAxis::Vertical,
            }))
        );
    }

    #[test]
    fn parses_action_focus_workspace() {
        assert_eq!(
            parse_ok(["nekoland-msg", "action", "focus-workspace", "2"]),
            ParsedAction::Request(IpcCommand::Action(ActionCommand::FocusWorkspace {
                workspace: "2".to_owned(),
            }))
        );
    }

    #[test]
    fn parses_action_focus_window() {
        assert_eq!(
            parse_ok(["nekoland-msg", "action", "focus-window", "--id", "77"]),
            ParsedAction::Request(IpcCommand::Action(ActionCommand::FocusWindow { id: 77 }))
        );
    }

    #[test]
    fn parses_action_spawn() {
        assert_eq!(
            parse_ok(["nekoland-msg", "action", "spawn", "foot", "--server"]),
            ParsedAction::Request(IpcCommand::Action(ActionCommand::Spawn {
                command: vec!["foot".to_owned(), "--server".to_owned()],
            }))
        );
    }

    #[test]
    fn parses_action_switch_keyboard_layout_variants() {
        assert_eq!(
            parse_ok(["nekoland-msg", "action", "switch-keyboard-layout-next"]),
            ParsedAction::Request(IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutNext))
        );
        assert_eq!(
            parse_ok(["nekoland-msg", "action", "switch-keyboard-layout-prev"]),
            ParsedAction::Request(IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutPrev))
        );
        assert_eq!(
            parse_ok(["nekoland-msg", "action", "switch-keyboard-layout-name", "de"]),
            ParsedAction::Request(IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutByName {
                name: "de".to_owned(),
            }))
        );
        assert_eq!(
            parse_ok(["nekoland-msg", "action", "switch-keyboard-layout-index", "2"]),
            ParsedAction::Request(IpcCommand::Action(ActionCommand::SwitchKeyboardLayoutByIndex {
                index: 2,
            }))
        );
    }

    #[test]
    fn parses_action_reload_config() {
        assert_eq!(
            parse_ok(["nekoland-msg", "action", "reload-config"]),
            ParsedAction::Request(IpcCommand::Action(ActionCommand::ReloadConfig))
        );
    }

    #[test]
    fn parses_action_quit() {
        assert_eq!(
            parse_ok(["nekoland-msg", "action", "quit"]),
            ParsedAction::Request(IpcCommand::Action(ActionCommand::Quit))
        );
    }

    #[test]
    fn parses_popup_dismiss() {
        assert_eq!(
            parse_ok(["nekoland-msg", "popup", "dismiss", "9"]),
            ParsedAction::Request(IpcCommand::Popup(PopupCommand::Dismiss { surface_id: 9 }))
        );
    }

    #[test]
    fn parses_popup_subscription() {
        assert_eq!(
            parse_ok(["nekoland-msg", "subscribe", "popup"]),
            ParsedAction::Subscribe(SubscriptionCommand {
                subscription: IpcSubscription {
                    topic: SubscriptionTopic::Popup,
                    include_payloads: true,
                    events: Vec::new(),
                },
                output_mode: SubscriptionOutputMode::Pretty,
            })
        );
    }

    #[test]
    fn parses_keyboard_layout_subscription() {
        assert_eq!(
            parse_ok(["nekoland-msg", "subscribe", "keyboard-layout"]),
            ParsedAction::Subscribe(SubscriptionCommand {
                subscription: IpcSubscription {
                    topic: SubscriptionTopic::KeyboardLayout,
                    include_payloads: true,
                    events: Vec::new(),
                },
                output_mode: SubscriptionOutputMode::Pretty,
            })
        );
    }

    #[test]
    fn parses_command_subscription() {
        assert_eq!(
            parse_ok(["nekoland-msg", "subscribe", "command", "--event", "command_*"]),
            ParsedAction::Subscribe(SubscriptionCommand {
                subscription: IpcSubscription {
                    topic: SubscriptionTopic::Command,
                    include_payloads: true,
                    events: vec!["command_*".to_owned()],
                },
                output_mode: SubscriptionOutputMode::Pretty,
            })
        );
    }

    #[test]
    fn parses_config_subscription() {
        assert_eq!(
            parse_ok(["nekoland-msg", "subscribe", "config", "--event", "config_changed"]),
            ParsedAction::Subscribe(SubscriptionCommand {
                subscription: IpcSubscription {
                    topic: SubscriptionTopic::Config,
                    include_payloads: true,
                    events: vec!["config_changed".to_owned()],
                },
                output_mode: SubscriptionOutputMode::Pretty,
            })
        );
    }

    #[test]
    fn parses_focus_subscription() {
        assert_eq!(
            parse_ok(["nekoland-msg", "subscribe", "focus", "--event", "focus_changed"]),
            ParsedAction::Subscribe(SubscriptionCommand {
                subscription: IpcSubscription {
                    topic: SubscriptionTopic::Focus,
                    include_payloads: true,
                    events: vec!["focus_changed".to_owned()],
                },
                output_mode: SubscriptionOutputMode::Pretty,
            })
        );
    }

    #[test]
    fn parses_clipboard_subscription() {
        assert_eq!(
            parse_ok(["nekoland-msg", "subscribe", "clipboard", "--event", "clipboard_changed"]),
            ParsedAction::Subscribe(SubscriptionCommand {
                subscription: IpcSubscription {
                    topic: SubscriptionTopic::Clipboard,
                    include_payloads: true,
                    events: vec!["clipboard_changed".to_owned()],
                },
                output_mode: SubscriptionOutputMode::Pretty,
            })
        );
    }

    #[test]
    fn parses_primary_selection_subscription() {
        assert_eq!(
            parse_ok([
                "nekoland-msg",
                "subscribe",
                "primary-selection",
                "--event",
                "primary_selection_changed",
            ]),
            ParsedAction::Subscribe(SubscriptionCommand {
                subscription: IpcSubscription {
                    topic: SubscriptionTopic::PrimarySelection,
                    include_payloads: true,
                    events: vec!["primary_selection_changed".to_owned()],
                },
                output_mode: SubscriptionOutputMode::Pretty,
            })
        );
    }

    #[test]
    fn parses_tree_subscription_without_payloads() {
        assert_eq!(
            parse_ok(["nekoland-msg", "subscribe", "tree", "--no-payloads"]),
            ParsedAction::Subscribe(SubscriptionCommand {
                subscription: IpcSubscription {
                    topic: SubscriptionTopic::Tree,
                    include_payloads: false,
                    events: Vec::new(),
                },
                output_mode: SubscriptionOutputMode::Pretty,
            })
        );
    }

    #[test]
    fn parses_output_subscription_as_jsonl() {
        assert_eq!(
            parse_ok(["nekoland-msg", "subscribe", "output", "--jsonl", "--no-payloads"]),
            ParsedAction::Subscribe(SubscriptionCommand {
                subscription: IpcSubscription {
                    topic: SubscriptionTopic::Output,
                    include_payloads: false,
                    events: Vec::new(),
                },
                output_mode: SubscriptionOutputMode::Jsonl,
            })
        );
    }

    #[test]
    fn parses_completion_subcommand() {
        assert_eq!(
            parse_ok(["nekoland-msg", "completion", "bash"]),
            ParsedAction::Completion(CompletionShellArg::Bash)
        );
        assert_eq!(
            parse_ok(["nekoland-msg", "completion", "fish"]),
            ParsedAction::Completion(CompletionShellArg::Fish)
        );
    }

    #[test]
    fn parses_all_subscription_with_multiple_event_filters() {
        assert_eq!(
            parse_ok([
                "nekoland-msg",
                "subscribe",
                "all",
                "--event",
                "workspaces_*",
                "--event",
                "tree_*",
                "--jsonl",
            ]),
            ParsedAction::Subscribe(SubscriptionCommand {
                subscription: IpcSubscription {
                    topic: SubscriptionTopic::All,
                    include_payloads: true,
                    events: vec!["workspaces_*".to_owned(), "tree_*".to_owned()],
                },
                output_mode: SubscriptionOutputMode::Jsonl,
            })
        );
    }

    #[test]
    fn rejects_missing_subscription_event_name() {
        assert!(parse_cli_from(["nekoland-msg", "subscribe", "workspace", "--event"]).is_err());
    }

    #[test]
    fn deduplicates_repeated_subscription_event_filters() {
        assert_eq!(
            parse_ok([
                "nekoland-msg",
                "subscribe",
                "workspace",
                "--event",
                "workspaces_changed",
                "--event",
                "workspaces_changed",
            ]),
            ParsedAction::Subscribe(SubscriptionCommand {
                subscription: IpcSubscription {
                    topic: SubscriptionTopic::Workspace,
                    include_payloads: true,
                    events: vec!["workspaces_changed".to_owned()],
                },
                output_mode: SubscriptionOutputMode::Pretty,
            })
        );
    }

    #[test]
    fn rejects_conflicting_subscription_output_flags() {
        assert!(
            parse_cli_from(["nekoland-msg", "subscribe", "workspace", "--pretty", "--jsonl",])
                .is_err()
        );
    }

    #[test]
    fn rejects_json_outside_subscription_help() {
        assert!(parse_cli_from(["nekoland-msg", "subscribe", "workspace", "--json"]).is_err());
    }

    #[test]
    fn recognizes_subscription_help_forms() {
        assert_eq!(
            parse_ok(["nekoland-msg", "subscribe", "--help"]),
            ParsedAction::SubscriptionHelp(HelpOutputMode::Text)
        );
        assert_eq!(
            parse_ok(["nekoland-msg", "help", "subscribe"]),
            ParsedAction::SubscriptionHelp(HelpOutputMode::Text)
        );
        assert_eq!(
            parse_ok(["nekoland-msg", "subscribe", "--help", "--json"]),
            ParsedAction::SubscriptionHelp(HelpOutputMode::Json)
        );
        assert_eq!(
            parse_ok(["nekoland-msg", "help", "subscribe", "--json"]),
            ParsedAction::SubscriptionHelp(HelpOutputMode::Json)
        );
    }

    #[test]
    fn subscription_help_lists_topics_and_known_events() {
        let Ok(help) = render_subscription_help(HelpOutputMode::Text) else {
            panic!("text subscription help should render successfully");
        };

        assert!(help.contains("Topics:"));
        assert!(help.contains("window"));
        assert!(help.contains("Known events:"));
        assert!(help.contains("window_created"));
        assert!(help.contains("prefix wildcard: window_*"));
    }

    #[test]
    fn subscription_help_json_lists_topics_and_known_events() {
        let Ok(help) = render_subscription_help(HelpOutputMode::Json) else {
            panic!("json subscription help should render successfully");
        };
        let Ok(help) = serde_json::from_str::<Value>(&help) else {
            panic!("json subscription help should decode as JSON");
        };
        let Some(known_events) = help["known_events"].as_array() else {
            panic!("known_events should be an array");
        };

        assert_eq!(help["topics"][0], "window");
        assert!(known_events.iter().any(|event| event == "window_created"));
        assert_eq!(help["patterns"]["prefix_wildcard_example"], "window_*");
    }

    #[test]
    fn completion_output_contains_command_tree() {
        let Ok(completion) = render_completion(CompletionShellArg::Bash) else {
            panic!("bash completion should render");
        };

        assert!(completion.contains("nekoland-msg"));
        assert!(completion.contains("subscribe"));
        assert!(completion.contains("completion"));
    }
}
