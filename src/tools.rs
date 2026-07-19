use std::fs;
use serde_json::{Map, Value};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Tool {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

pub fn read_file(input: &Map<String, Value>) -> Result<String, Box<dyn std::error::Error>> {
    let file_path = input.get("file_path").and_then(|x| x.as_str());
    let Some(path) = file_path else {
        return Err("file_input missing or not a string".into());
    };
    let bytes = fs::read(path)?;
    let result = String::from_utf8(bytes)?;
    return Ok(result);
}

pub fn write_file(input: &Map<String, Value>) -> Result<String, Box<dyn std::error::Error>> {
    let file_path = input.get("file_path").and_then(|x| x.as_str());
    let content = input.get("content").and_then(|x| x.as_str());
    let Some(path) = file_path else {
        return Err("file_input missing or not a string".into());
    };
    let Some(content) = content else {
        return Err("content missing or not a string".into());
    };
    let target_path = Path::new(&path);
    if let Some(parent_path) = target_path.parent() {
        fs::create_dir_all(parent_path)?;
    }
    let _ = fs::write(path, content)?;
    return Ok("Created the file".to_string());
}

pub fn execute_bash(input: &Map<String, Value>) -> Result<String, Box<dyn std::error::Error>> {
    let Some(command) = input.get("command").and_then(|x| x.as_str()) else {
        return Err("Command attribute missing!".into());
    };

    let Ok(result) = Command::new("sh").arg("-c").arg(command).output() else {
        return Err("Failed to execute command!".into());
    };

    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).into());
    }

    return Ok(String::from_utf8_lossy(&result.stdout).to_string());
}
