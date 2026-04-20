//! OAuth CLI tool — mirrors pi-mono's `packages/ai/src/cli.ts`.
//!
//! Provides `login` and `list` commands for managing OAuth provider credentials.
//! Credentials are stored in `auth.json` in the current working directory.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::Path;

use crate::utils::oauth::{
    OAuthAuthInfo, OAuthCredentials, OAuthLoginCallbacks, OAuthPrompt,
    get_oauth_provider, get_oauth_providers,
};

const AUTH_FILE: &str = "auth.json";

// ---------------------------------------------------------------------------
// Persist / load auth
// ---------------------------------------------------------------------------

/// Auth entry stored on disk.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct AuthEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(flatten)]
    pub credentials: OAuthCredentials,
}

/// Load the auth map from `auth.json`. Returns an empty map if the file does
/// not exist or cannot be parsed.
pub fn load_auth() -> HashMap<String, AuthEntry> {
    if !Path::new(AUTH_FILE).exists() {
        return HashMap::new();
    }
    match std::fs::read_to_string(AUTH_FILE) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Persist the auth map to `auth.json`.
pub fn save_auth(auth: &HashMap<String, AuthEntry>) {
    let json = serde_json::to_string_pretty(auth).unwrap_or_default();
    let _ = std::fs::write(AUTH_FILE, json.as_bytes());
}

// ---------------------------------------------------------------------------
// login command
// ---------------------------------------------------------------------------

/// Run the login flow for the given provider ID.
pub async fn login(provider_id: &str) -> Result<(), String> {
    let provider = get_oauth_provider(provider_id)
        .ok_or_else(|| format!("Unknown provider: {provider_id}"))?;

    let callbacks = OAuthLoginCallbacks {
        on_auth: Box::new(|info: OAuthAuthInfo| {
            println!("\nOpen this URL in your browser:\n{}", info.url);
            if let Some(instructions) = info.instructions {
                println!("{instructions}");
            }
            println!();
        }),
        on_prompt: Box::new(|p: OAuthPrompt| {
            Box::pin(async move {
                let placeholder_suffix = p
                    .placeholder
                    .as_deref()
                    .map(|ph| format!(" ({ph})"))
                    .unwrap_or_default();
                let question = format!("{}{placeholder_suffix}: ", p.message);
                read_line_prompt(&question).await
            })
        }),
        on_progress: Some(Box::new(|msg: String| {
            println!("{msg}");
        })),
        on_manual_code_input: None,
    };

    let credentials = provider.login(callbacks).await.map_err(|e| e.to_string())?;

    let mut auth = load_auth();
    auth.insert(
        provider_id.to_owned(),
        AuthEntry {
            entry_type: "oauth".to_owned(),
            credentials,
        },
    );
    save_auth(&auth);

    println!("\nCredentials saved to {AUTH_FILE}");
    Ok(())
}

// ---------------------------------------------------------------------------
// CLI entry point
// ---------------------------------------------------------------------------

/// Run the CLI. `args` should be the arguments after the binary name
/// (analogous to `process.argv.slice(2)` in Node).
pub async fn run(args: &[String]) -> i32 {
    let providers = get_oauth_providers();
    let command = args.first().map(String::as_str);

    // ---- help ----
    if command.is_none()
        || command == Some("help")
        || command == Some("--help")
        || command == Some("-h")
    {
        let provider_list: String = providers
            .iter()
            .map(|p| format!("  {:<20} {}", p.id(), p.name()))
            .collect::<Vec<_>>()
            .join("\n");
        println!(
            "Usage: pi-ai <command> [provider]\n\n\
             Commands:\n  \
             login [provider]  Login to an OAuth provider\n  \
             list              List available providers\n\n\
             Providers:\n\
             {provider_list}\n\n\
             Examples:\n  \
             pi-ai login              # interactive provider selection\n  \
             pi-ai login anthropic    # login to specific provider\n  \
             pi-ai list               # list providers\n"
        );
        return 0;
    }

    // ---- list ----
    if command == Some("list") {
        println!("Available OAuth providers:\n");
        for p in &providers {
            println!("  {:<20} {}", p.id(), p.name());
        }
        return 0;
    }

    // ---- login ----
    if command == Some("login") {
        let provider_id: String = if let Some(id) = args.get(1) {
            id.clone()
        } else {
            // Interactive provider selection
            println!("Select a provider:\n");
            for (i, p) in providers.iter().enumerate() {
                println!("  {}. {}", i + 1, p.name());
            }
            println!();

            let choice = read_line_prompt(&format!("Enter number (1-{}): ", providers.len())).await;
            let index: usize = match choice.trim().parse::<usize>() {
                Ok(n) if n >= 1 && n <= providers.len() => n - 1,
                _ => {
                    eprintln!("Invalid selection");
                    return 1;
                }
            };
            providers[index].id().to_owned()
        };

        // Validate provider ID
        if !providers.iter().any(|p| p.id() == provider_id) {
            eprintln!("Unknown provider: {provider_id}");
            eprintln!("Use 'pi-ai list' to see available providers");
            return 1;
        }

        println!("Logging in to {provider_id}...");
        match login(&provider_id).await {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("Error: {e}");
                1
            }
        }
    } else {
        eprintln!("Unknown command: {}", command.unwrap_or(""));
        eprintln!("Use 'pi-ai --help' for usage");
        1
    }
}

// ---------------------------------------------------------------------------
// Helper: synchronous stdin prompt (returns a future for use in callbacks)
// ---------------------------------------------------------------------------

async fn read_line_prompt(question: &str) -> String {
    print!("{question}");
    let _ = io::stdout().flush();
    let stdin = io::stdin();
    let mut line = String::new();
    let _ = stdin.lock().read_line(&mut line);
    line.trim_end_matches(['\n', '\r']).to_owned()
}
