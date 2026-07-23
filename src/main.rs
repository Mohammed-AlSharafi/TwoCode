mod agent;
mod interface;
mod specs;
mod tools;
mod events;

use tools::{execute_bash, read_file, write_file};

use specs::{bash_tool_spec, read_tool_spec, write_tool_spec};

use async_openai::{Client, config::OpenAIConfig};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::{env, process};
use tokio::sync::mpsc;

use crate::agent::Agent;
use crate::events::DisplayEvent;
use crate::interface::Interface;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().expect("Failed to load .env file");

    let base_url =
        env::var("BASE_URL").unwrap_or_else(|_| "https://api.groq.com/openai/v1".to_string());

    let api_key = env::var("API_KEY").unwrap_or_else(|_| {
        eprintln!("API_KEY is not set");
        process::exit(1);
    });

    let config = OpenAIConfig::new()
        .with_api_base(base_url)
        .with_api_key(api_key);

    let client = Client::with_config(config);

    let model = "qwen/qwen3.6-27b";

    let mut functions: HashMap<
        String,
        fn(&Map<String, Value>) -> Result<String, Box<dyn std::error::Error>>,
    > = HashMap::new();
    functions.insert("Read".to_string(), read_file);
    functions.insert("Write".to_string(), write_file);
    functions.insert("Bash".to_string(), execute_bash);

    let messages: Vec<Value> = Vec::<Value>::new();

    let specs = vec![bash_tool_spec, write_tool_spec, read_tool_spec];
    
    let (event_tx, event_rx) = mpsc::unbounded_channel::<DisplayEvent>();

    let mut agent = Agent::with_history(client, model.to_string(), messages, specs, functions, event_tx);
    let mut interface = Interface::new(event_rx);

    let _ = interface.run(&mut agent).await?;

    // let (agent_result, interface_result) = tokio::join!(agent.run(), interface.run());

    // interface_result?;
    // agent_result?;

    Ok(())
}
