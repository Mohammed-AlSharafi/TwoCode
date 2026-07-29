use std::collections::{HashMap, hash_map::Entry};

use async_openai::{Client, config::OpenAIConfig};
use futures::StreamExt;
use serde_json::{Map, Value, json};
use tokio::sync::mpsc::UnboundedSender;

use crate::events::DisplayEvent;
use crate::tools::Tool;

pub struct Agent {
    client: Client<OpenAIConfig>,
    model: String,
    messages: Vec<Value>,
    specs: Vec<fn() -> Value>,
    functions:
        HashMap<String, fn(&Map<String, Value>) -> Result<String, Box<dyn std::error::Error>>>,
    event_tx: UnboundedSender<DisplayEvent>,
}

impl Agent {
    pub fn with_history(
        client: Client<OpenAIConfig>,
        model: String,
        messages: Vec<Value>,
        specs: Vec<fn() -> Value>,
        functions: HashMap<
            String,
            fn(&Map<String, Value>) -> Result<String, Box<dyn std::error::Error>>,
        >,
        event_tx: UnboundedSender<DisplayEvent>,
    ) -> Self {
        Agent {
            client,
            model,
            messages,
            specs,
            functions,
            event_tx,
        }
    }

    pub async fn agent_loop(
        &mut self,
        user_input: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(input) = user_input {
            self.messages.push(json!({
              "role": "user",
              "content": input
            }));
        }
        loop {
            let tools_specs: Vec<Value> = self.specs.iter().map(|x| x()).collect();
            let mut stream = self
                .client
                .chat()
                .create_stream_byot::<Value, Value>(json!({
                    "messages": &self.messages,
                    "model": &self.model,
                    "tools": tools_specs,
                    "stream": true,
                    "max_completion_tokens": 16384,
                }))
                .await?;

            let mut content = Vec::<String>::new();

            let mut tools = HashMap::<u64, Tool>::new();
            let mut finish_reason: Option<String> = None;

            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        finish_reason = Some("stream_error".to_string());
                        self.display_error(e.to_string()).ok();
                        //keep whatever content we've accumulated so far
                        break;
                    }
                };

                let delta = &chunk["choices"][0]["delta"];

                if let Some(reason) = chunk["choices"][0]["finish_reason"].as_str() {
                    finish_reason = Some(reason.to_string());
                }

                if let Some(reasoning_str_chunk) = delta["reasoning"].as_str() {
                    self.event_tx
                        .send(DisplayEvent::Reasoning(reasoning_str_chunk.to_string()))
                        .ok();
                }

                if let Some(content_str_chunk) = delta["content"].as_str() {
                    content.push(content_str_chunk.to_string());
                    self.event_tx
                        .send(DisplayEvent::Content(content_str_chunk.to_string()))
                        .ok();
                }

                if let Some(tool_calls) = delta["tool_calls"].as_array() {
                    for tool_call in tool_calls {
                        let Some(tool_index) = tool_call["index"].as_u64() else {
                            eprintln!("{}", "[Tool Accumilation]: Tool index not found!");
                            break;
                        };

                        let Some(arguments) = tool_call["function"]["arguments"].as_str() else {
                            eprintln!("{}", "[Tool Accumilation]: Arguments not found!");
                            break;
                        };

                        match tools.entry(tool_index) {
                            Entry::Occupied(mut entry) => entry.get_mut().arguments += arguments,
                            Entry::Vacant(entry) => {
                                let Some(tool_id) = tool_call["id"].as_str() else {
                                    eprintln!("{}", "[Tool Accumilation]: Tool id not found!");
                                    break;
                                };
                                let Some(tool_name) = tool_call["function"]["name"].as_str() else {
                                    eprintln!("{}", "[Tool Accumilation]: Tool name not found!");
                                    break;
                                };

                                entry.insert(Tool::new(
                                    tool_id.to_string(),
                                    tool_name.to_string(),
                                    arguments.to_string(),
                                    None,
                                ));
                            }
                        }
                    }
                }
            }

            if let Some(ref reason) = finish_reason {
                if reason != "tool_calls" {
                    let stop_error = match reason.as_str(){
                    "length" => "Response truncated: max tokens reached. Consider increasing max_completion_tokens.".to_string(),
                    "content_filter" => "Content was omitted because it was flagged by the moderation/safety filters.".to_string(),
                    "stop" => break,
                    other => format!("Stream finished with reason: {}", other)
                };
                    self.display_error(stop_error).ok();
                    break;
                }
            }

            if tools.is_empty() {
                break;
            }

            let mut sorted_tools: Vec<(&u64, &Tool)> = tools.iter().collect();

            sorted_tools.sort_by_key(|(index, _)| **index);

            let tool_calls: Vec<Value> = sorted_tools
                .iter()
                .map(|(_, tool)| {
                    json!({
                        "id": &tool.id,
                        "type": "function",
                        "function": {
                            "name": &tool.name,
                            "arguments": &tool.arguments
                        }
                    })
                })
                .collect();

            self.messages.push(json!({
                "role": "assistant",
                "content": if content.is_empty() { Value::Null } else { json!(content.join("")) },
                "tool_calls": tool_calls
            }));
            // if and else need to be blocks, and they must return same type.

            for (_, tool) in sorted_tools {
                self.event_tx
                    .send(DisplayEvent::ToolCall(tool.clone()))
                    .ok();
                let Some(function) = self.functions.get(tool.name.as_str()) else {
                    eprintln!("{}", "[Tool Execution]: Tool not found!");
                    continue;
                };

                let Ok(args_map) = serde_json::from_str::<Value>(&tool.arguments) else {
                    eprintln!("{}", "[Tool Execution]: Arguments parse failed!");
                    continue;
                };

                let Some(args_map_obj) = args_map.as_object() else {
                    eprintln!("{}", "[Tool Execution]: Arguments parse failed!");
                    continue;
                };

                let Ok(tool_result) = function(args_map_obj) else {
                    eprintln!("{}", "[Tool Execution]: Tool calling returned an error!");
                    continue;
                };

                self.messages.push(json!({
                    "role": "tool",
                    "tool_call_id": tool.id,
                    "content": tool_result
                }));
            }
        }
        Ok(())
    }

    fn display_error(&self, error: String) -> Result<(), Box<dyn std::error::Error>> {
        self.event_tx.send(DisplayEvent::Error(error))?;
        Ok(())
    }
}
