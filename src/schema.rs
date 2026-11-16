//! Stable machine-readable CLI contract.
use serde_json::{Value, json};
pub const CLISPEC_VERSION: &str = "0.3";
pub fn contract() -> Value {
    let month = json!({"name":"--month","type":"string","required":true,"description":"Calendar month YYYY-MM."});
    let file =
        json!({"name":"file","type":"string","required":true,"description":"JSON input file."});
    let mut contract = json!({
        "clispec":CLISPEC_VERSION,"name":env!("CARGO_PKG_NAME"),"version":env!("CARGO_PKG_VERSION"),"description":env!("CARGO_PKG_DESCRIPTION"),
        "global_args":[{"name":"--output","type":"string","enum":["auto","json","text"],"default":"auto","description":"Text on a terminal; JSON when piped."}],
        "commands":[
            {"name":"timesheet","description":"Read a month and its project day minutes.","mutating":false,"stability":"stable","args":[month.clone()],"output_fields":[{"name":"base_url","type":"string"},{"name":"employee_id","type":"string"},{"name":"month","type":"string"},{"name":"total_minutes","type":"integer"},{"name":"projects","type":"array"}]},
            {"name":"projects","description":"List available booking activities.","mutating":false,"stability":"stable","args":[month]},
            {"name":"plan","description":"Create a plan from fresh booked hours and actual work. Nearest 4/8 hours, 6h rounds up, explicit overrides supported.","mutating":false,"stability":"stable","args":[file.clone()]},
            {"name":"apply","description":"Apply an explicitly approved plan; reject stale or locked changes and verify after saving.","mutating":true,"stability":"stable","args":[file.clone(),{"name":"--yes","type":"boolean","description":"Authorize this plan's writes."},{"name":"--dry-run","type":"boolean","description":"Validate locally without requests."}]},
            {"name":"balance","description":"Calculate actual minus booked minutes plus opening carry, offline.","mutating":false,"stability":"stable","args":[file]},
            {"name":"auth","description":"Credential availability.","mutating":false,"stability":"stable","subcommands":[{"name":"status","description":"Local credential status without network or secrets.","mutating":false,"stability":"stable"}]},
            {"name":"schema","description":"Print this contract.","mutating":false,"stability":"stable"},
            {"name":"completions","description":"Generate shell completion script.","mutating":false,"stability":"stable","args":[{"name":"shell","type":"string","required":true,"enum":["bash","zsh","fish","powershell","elvish"]}]}
        ],
        "errors":[
            {"kind":"usage","exit_code":3,"retryable":false,"description":"Invalid command arguments."},
            {"kind":"invalid_input","exit_code":1,"retryable":false,"description":"Invalid dates, durations, JSON, or plan."},
            {"kind":"auth","exit_code":4,"retryable":false,"description":"Missing or rejected credentials."},
            {"kind":"network","exit_code":1,"retryable":false,"description":"Network request failed; inspect before retrying writes."},
            {"kind":"protocol","exit_code":1,"retryable":false,"description":"Unexpected or inconsistent Yoobi response."},
            {"kind":"conflict","exit_code":5,"retryable":false,"description":"Stale plan, different account, or locked date."},
            {"kind":"io","exit_code":1,"retryable":false,"description":"Local input file could not be read."}
        ]
    });
    contract["output"] = json!({"tty":"text","piped":"json"});
    let mut commands = contract["commands"].as_array().unwrap().clone();
    commands.retain(|c| c["name"] != "auth");
    commands.insert(0,json!({"name":"auth status","description":"Inspect local credential configuration without network access.","mutating":false}));
    commands.push(json!({"name":"auth login","description":"Verify credentials with Yoobi and save the password in the OS credential store.","mutating":true,"args":[
        {"name":"--base-url","type":"string","description":"Yoobi HTTPS origin; prompts if missing"},
        {"name":"--username","type":"string","description":"Yoobi username; prompts if missing"},
        {"name":"--password-stdin","type":"boolean","description":"Read the password from stdin; conflicts with environment password sources"}
    ]}));
    commands.push(json!({"name":"auth logout","description":"Remove this CLI's saved login from local metadata and the OS credential store. Environment credentials remain available.","mutating":true}));
    commands.push(json!({"name":"tui","description":"Keyboard timesheet browser with daily detail and booking-plan preview. Never saves bookings.","effects":"read_only","output_kind":"opaque","media_type":"text/plain","requires_tty":true,"args":[
        {"name":"--month","type":"string","description":"YYYY-MM; defaults to current month"},
        {"name":"--plan","type":"path","description":"Read-only preview of an existing booking plan"},
        {"name":"--demo","type":"boolean","description":"Use clearly labelled offline sample data"},
        {"name":"--snapshot","type":"boolean","description":"Render one plain-text screen; requires --demo"},
        {"name":"--width","type":"integer","description":"Snapshot width, 40–240", "default":120},
        {"name":"--height","type":"integer","description":"Snapshot height, 16–80", "default":36}
    ],"extensions":{"snapshot_requires_demo":true,"saves_bookings":false}}));
    for cmd in &mut commands {
        let name = cmd["name"].as_str().unwrap().to_owned();
        cmd.as_object_mut().unwrap().remove("subcommands");
        cmd["effects"] = json!(
            if matches!(name.as_str(), "apply" | "auth login" | "auth logout") {
                "idempotent"
            } else {
                "read_only"
            }
        );
        if name == "completions" {
            cmd["output_kind"] = json!("opaque");
            cmd["media_type"] = json!("text/plain");
            continue;
        }
        if name == "tui" {
            continue;
        }
        cmd["cardinality"] = json!("single");
        let fields = match name.as_str() {
            "timesheet" => json!([
                ["base_url", "string"],
                ["employee_id", "string"],
                ["month", "string"],
                ["total_minutes", "integer"],
                ["projects", "array"]
            ]),
            "projects" => json!([
                ["month", "string"],
                ["employee_id", "string"],
                ["projects", "array"]
            ]),
            "plan" => json!([
                ["version", "integer"],
                ["base_url", "string"],
                ["employee_id", "string"],
                ["month", "string"],
                ["project_id", "string"],
                ["opening_balance_minutes", "integer"],
                ["days", "array"]
            ]),
            "apply" => json!([
                ["verified", "boolean"],
                ["changed_days", "integer"],
                ["timesheet", "object"],
                ["dry_run", "boolean"],
                ["plan", "object"],
                ["balance", "object"]
            ]),
            "balance" => json!([
                ["actual_minutes", "integer"],
                ["booked_minutes", "integer"],
                ["opening_balance_minutes", "integer"],
                ["balance_minutes", "integer"]
            ]),
            "auth login" => json!([
                ["saved", "boolean"],
                ["verified", "boolean"],
                ["source", "string"],
                ["cleanup_pending", "boolean"]
            ]),
            "auth logout" => json!([["saved", "boolean"], ["removed", "boolean"]]),
            "auth status" => json!([
                ["configured", "boolean"],
                ["source", "string"],
                ["verified", "boolean"]
            ]),
            _ => json!([
                ["clispec", "string"],
                ["name", "string"],
                ["version", "string"],
                ["commands", "array"],
                ["errors", "array"]
            ]),
        };
        cmd["output_fields"] = json!(
            fields
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    let mut field = json!({"name":f[0],"type":f[1]});
                    if f[1] == "array" {
                        field["items"] = json!({"type":"object"});
                    }
                    field
                })
                .collect::<Vec<_>>()
        );
    }
    contract["commands"] = json!(commands);
    contract["errors"].as_array_mut().unwrap().push(json!({"kind":"tty_required","exit_code":6,"retryable":false,"description":"Interactive command requires terminal stdin and stdout."}));
    contract
}
pub fn contract_json() -> String {
    serde_json::to_string_pretty(&contract()).expect("contract serializes")
}
