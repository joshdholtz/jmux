use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::AgentState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetStatusParams {
    pub state: String,
    pub message: Option<String>,
}

impl Request {
    pub fn parse(line: &str) -> Result<Self> {
        Ok(serde_json::from_str(line)?)
    }
}

pub fn parse_set_status(params: &Value) -> Result<AgentState> {
    let p: SetStatusParams = serde_json::from_value(params.clone())?;
    match p.state.as_str() {
        "idle" => Ok(AgentState::Idle),
        "working" => Ok(AgentState::Working { message: p.message }),
        "waiting" => Ok(AgentState::Waiting { message: p.message }),
        "error" => Ok(AgentState::Error { message: p.message }),
        other => Err(anyhow!("unknown state: {}", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_set_status_working() {
        let json =
            r#"{"method":"set-status","params":{"state":"working","message":"Editing auth.rs"}}"#;
        let req = Request::parse(json).unwrap();
        assert_eq!(req.method, "set-status");
        let state = parse_set_status(&req.params).unwrap();
        assert!(
            matches!(state, crate::AgentState::Working { message: Some(m) } if m == "Editing auth.rs")
        );
    }

    #[test]
    fn parse_valid_set_status_idle() {
        let json = r#"{"method":"set-status","params":{"state":"idle"}}"#;
        let req = Request::parse(json).unwrap();
        let state = parse_set_status(&req.params).unwrap();
        assert_eq!(state, crate::AgentState::Idle);
    }

    #[test]
    fn parse_valid_set_status_waiting_no_message() {
        let json = r#"{"method":"set-status","params":{"state":"waiting"}}"#;
        let req = Request::parse(json).unwrap();
        let state = parse_set_status(&req.params).unwrap();
        assert!(matches!(
            state,
            crate::AgentState::Waiting { message: None }
        ));
    }

    #[test]
    fn parse_set_status_error_state() {
        let json = r#"{"method":"set-status","params":{"state":"error","message":"build failed"}}"#;
        let req = Request::parse(json).unwrap();
        let state = parse_set_status(&req.params).unwrap();
        assert!(
            matches!(state, crate::AgentState::Error { message: Some(m) } if m == "build failed")
        );
    }

    #[test]
    fn parse_invalid_json_returns_err() {
        assert!(Request::parse("not json").is_err());
    }

    #[test]
    fn parse_unknown_state_returns_err() {
        let params = serde_json::json!({"state": "flying"});
        assert!(parse_set_status(&params).is_err());
    }

    #[test]
    fn parse_flash_method() {
        let json = r#"{"method":"flash","params":{}}"#;
        let req = Request::parse(json).unwrap();
        assert_eq!(req.method, "flash");
    }
}
