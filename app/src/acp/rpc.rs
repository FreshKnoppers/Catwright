//! Line-based JSON-RPC 2.0 codec. Pure functions, no I/O: one message per
//! line (UTF-8, newline-delimited) on the agent's stdin/stdout.

use crate::i18n::{tr, trf};
use serde_json::{json, Value};

pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const AUTH_REQUIRED: i64 = -32000;

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// Answer to one of OUR requests (our ids are u64).
    Response {
        id: u64,
        result: Result<Value, RpcError>,
    },
    /// Request FROM the agent (id may be number or string; Vibe starts at 0).
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// Notification (no id).
    Notification { method: String, params: Value },
}

/// Parse one line into a typed message. Missing `params` becomes
/// `Value::Null`. A non-u64 `id` on a response is an error.
pub fn parse_incoming(line: &str) -> Result<Incoming, String> {
    let v: Value = serde_json::from_str(line)
        .map_err(|e| trf("Line is not valid JSON: {error}", &[("error", &e)]))?;
    let obj = v
        .as_object()
        .ok_or_else(|| tr("JSON-RPC message is not an object").to_string())?;
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    if let Some(m) = obj.get("method") {
        let method = m
            .as_str()
            .ok_or_else(|| tr("JSON-RPC method is not a string").to_string())?
            .to_string();
        return match obj.get("id") {
            Some(id) if !id.is_null() => Ok(Incoming::Request {
                id: id.clone(),
                method,
                params,
            }),
            _ => Ok(Incoming::Notification { method, params }),
        };
    }
    if let Some(id) = obj.get("id") {
        let id = id
            .as_u64()
            .ok_or_else(|| tr("Response id is not a positive integer").to_string())?;
        if let Some(err) = obj.get("error") {
            let e = err
                .as_object()
                .ok_or_else(|| tr("JSON-RPC error is not an object").to_string())?;
            let code = e
                .get("code")
                .and_then(Value::as_i64)
                .ok_or_else(|| tr("JSON-RPC error.code is missing").to_string())?;
            let message = e
                .get("message")
                .and_then(Value::as_str)
                .ok_or_else(|| tr("JSON-RPC error.message is missing").to_string())?
                .to_string();
            let data = e.get("data").cloned();
            return Ok(Incoming::Response {
                id,
                result: Err(RpcError {
                    code,
                    message,
                    data,
                }),
            });
        }
        if obj.contains_key("result") {
            let result = obj.get("result").cloned().unwrap_or(Value::Null);
            return Ok(Incoming::Response {
                id,
                result: Ok(result),
            });
        }
        return Err(tr("JSON-RPC message has neither method nor result/error").to_string());
    }
    Err(tr("JSON-RPC message has neither method nor id").to_string())
}

/// Exactly one line, no trailing `\n`, always with `"jsonrpc":"2.0"`.
pub fn encode_request(id: u64, method: &str, params: Value) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    }))
    .expect("JSON-RPC request serializes")
}

pub fn encode_notification(method: &str, params: Value) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    }))
    .expect("JSON-RPC notification serializes")
}

pub fn encode_result(id: &Value, result: Value) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
    .expect("JSON-RPC result serializes")
}

pub fn encode_error(id: &Value, code: i64, message: &str) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    }))
    .expect("JSON-RPC error serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_request_with_numeric_and_string_id() {
        let r = parse_incoming(
            r#"{"jsonrpc":"2.0","id":0,"method":"session/request_permission","params":{"a":1}}"#,
        )
        .unwrap();
        match r {
            Incoming::Request { id, method, params } => {
                assert_eq!(id, json!(0));
                assert_eq!(method, "session/request_permission");
                assert_eq!(params, json!({"a": 1}));
            }
            other => panic!("expected Request, got {other:?}"),
        }
        let r = parse_incoming(r#"{"jsonrpc":"2.0","id":"abc-1","method":"fs/read"}"#).unwrap();
        match r {
            Incoming::Request { id, method, params } => {
                assert_eq!(id, json!("abc-1"));
                assert_eq!(method, "fs/read");
                assert!(params.is_null(), "missing params must become Null");
            }
            other => panic!("expected Request, got {other:?}"),
        }
    }

    #[test]
    fn parses_notification_and_null_id_as_notification() {
        let r =
            parse_incoming(r#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#).unwrap();
        match r {
            Incoming::Notification { method, params } => {
                assert_eq!(method, "session/update");
                assert_eq!(params, json!({}));
            }
            other => panic!("expected Notification, got {other:?}"),
        }
        let r = parse_incoming(r#"{"jsonrpc":"2.0","id":null,"method":"x"}"#).unwrap();
        assert!(matches!(r, Incoming::Notification { .. }));
    }

    #[test]
    fn parses_response_ok_and_err() {
        let r = parse_incoming(r#"{"jsonrpc":"2.0","id":3,"result":{"ok":true}}"#).unwrap();
        match r {
            Incoming::Response { id, result } => {
                assert_eq!(id, 3);
                assert_eq!(result.unwrap(), json!({"ok": true}));
            }
            other => panic!("expected Response, got {other:?}"),
        }
        let r = parse_incoming(
            r#"{"jsonrpc":"2.0","id":4,"error":{"code":-32000,"message":"auth","data":{"x":1}}}"#,
        )
        .unwrap();
        match r {
            Incoming::Response { id, result } => {
                assert_eq!(id, 4);
                let e = result.unwrap_err();
                assert_eq!(
                    e,
                    RpcError {
                        code: -32000,
                        message: "auth".to_string(),
                        data: Some(json!({"x": 1})),
                    }
                );
            }
            other => panic!("expected Response, got {other:?}"),
        }
        // explicit null result is still a Response
        let r = parse_incoming(r#"{"jsonrpc":"2.0","id":5,"result":null}"#).unwrap();
        match r {
            Incoming::Response { result, .. } => assert_eq!(result.unwrap(), Value::Null),
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[test]
    fn rejects_garbage_and_bad_ids() {
        assert!(parse_incoming("not json").is_err());
        assert!(parse_incoming("").is_err());
        assert!(parse_incoming("[1,2]").is_err());
        assert!(parse_incoming(r#"{"jsonrpc":"2.0","id":"abc","result":{}}"#.trim()).is_err());
        assert!(parse_incoming(r#"{"jsonrpc":"2.0","id":1.5,"result":{}}"#).is_err());
        assert!(parse_incoming(r#"{"jsonrpc":"2.0","id":9}"#).is_err());
        assert!(parse_incoming(r#"{"jsonrpc":"2.0"}"#).is_err());
    }

    #[test]
    fn encoders_emit_single_line_jsonrpc2() {
        let lines = [
            encode_request(1, "initialize", json!({"a": 1})),
            encode_notification("session/cancel", json!({"sessionId": "s"})),
            encode_result(&json!(0), json!({"outcome": "cancelled"})),
            encode_error(&json!(7), METHOD_NOT_FOUND, "Method not found"),
        ];
        for line in lines {
            assert!(!line.contains('\n'), "encoder must not append newline");
            let v: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(v["jsonrpc"], "2.0");
        }
        let req: Value =
            serde_json::from_str(&encode_request(42, "session/new", json!({}))).unwrap();
        assert_eq!(req["id"], json!(42));
        assert_eq!(req["method"], "session/new");
        let err: Value =
            serde_json::from_str(&encode_error(&json!("x"), INVALID_PARAMS, "Invalid params"))
                .unwrap();
        assert_eq!(err["id"], json!("x"));
        assert_eq!(err["error"]["code"], json!(INVALID_PARAMS));
        let res: Value = serde_json::from_str(&encode_result(&json!(0), json!({}))).unwrap();
        assert!(res.get("result").is_some());
        assert!(res.get("error").is_none());
    }
}
