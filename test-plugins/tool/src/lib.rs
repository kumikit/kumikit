//! Test plugin: three tools for the tool gateway. `echo` (read) answers with `text`;
//! `cwd` (read) runs `fake-acp-agent cwd` in the calling thread's workspace and answers
//! with its output; `touch` (write) answers `touched <text>`. Built only with the public SDK.

use kumikit_tool::{Error, ErrorCode, Plugin, SpawnRequest, Tool, ToolCall, ToolResult, process};
use serde_json::Value;

struct Tools;

impl Plugin for Tools {
    fn activate() -> Result<(), Error> {
        Ok(())
    }

    fn deactivate() {}
}

impl Tool for Tools {
    fn call(call: ToolCall) -> Result<ToolResult, Error> {
        let input: Value = serde_json::from_str(&call.input).unwrap_or(Value::Null);
        let text = input
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match call.name.as_str() {
            "echo" => Ok(ToolResult {
                text: text.to_owned(),
                structured: Some(
                    serde_json::json!({ "echo": text, "thread": call.thread }).to_string(),
                ),
                is_error: false,
            }),
            "touch" => Ok(ToolResult::text(format!("touched {text}"))),
            "cwd" => {
                let child = process::spawn(&SpawnRequest {
                    program: "fake-acp-agent".into(),
                    args: vec!["cwd".into()],
                    cwd: None,
                    env: Vec::new(),
                    secret_env: Vec::new(),
                })?;
                child.close_stdin();
                let mut out = Vec::new();
                while let Some(chunk) = child.read_stdout(64 * 1024)? {
                    out.extend(chunk);
                }
                child.wait()?;
                Ok(ToolResult::text(
                    String::from_utf8_lossy(&out).trim().to_owned(),
                ))
            }
            other => Err(Error::new(ErrorCode::NotFound, format!("no tool {other}"))),
        }
    }
}

kumikit_tool::export_tool!(Tools);
