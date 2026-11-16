//! api/app.py 와 같은 HTTP 계약의 라우터와 EtaRequest 검증.
//!
//! axum 의 Json 추출기는 형식 오류에 400·415 를 돌려주므로 쓰지 않는다. 본문을 바이트로 받아 직접 파싱해
//! FastAPI(pydantic) 처럼 모든 입력 오류를 422 와 detail[] 로 돌려준다.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{Map, Value, json};

/// (거리 km, 대기 주문 수) → 예상 시간(분).
pub type Predict = Arc<dyn Fn(f64, f64) -> Result<f64, String> + Send + Sync>;

const MAX_BODY_BYTES: usize = 1 << 20;

#[derive(Clone)]
struct AppState {
    predict: Predict,
    model: &'static str,
}

/// api/app.py 와 같은 계약의 라우터를 만든다.
pub fn router(predict: Predict, model: &'static str) -> Router {
    Router::new()
        .route("/health", any(health))
        .route("/eta", any(eta))
        .fallback(|| async { detail(StatusCode::NOT_FOUND, "Not Found") })
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(AppState { predict, model })
}

async fn health(method: Method, State(st): State<AppState>) -> Response {
    if method != Method::GET {
        return method_not_allowed("GET");
    }
    Json(json!({"status": "ok", "model": st.model})).into_response()
}

async fn eta(
    method: Method,
    State(st): State<AppState>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    if method != Method::POST {
        return method_not_allowed("POST");
    }
    let raw = match body {
        Ok(b) => b,
        Err(rej) if rej.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return detail(StatusCode::PAYLOAD_TOO_LARGE, "Request body too large");
        }
        Err(_) => return detail(StatusCode::BAD_REQUEST, "요청 본문을 읽지 못했습니다."),
    };

    let content_type = headers.get(header::CONTENT_TYPE).map(|v| v.to_str().unwrap_or("\u{0}"));
    let (distance, orders) = match parse_eta_request(content_type, &raw) {
        Ok(v) => v,
        Err(errs) => return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"detail": errs}))).into_response(),
    };

    let minutes = match (st.predict)(distance, orders) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("예측 실패: {e}");
            return detail(StatusCode::INTERNAL_SERVER_ERROR, "예측에 실패했습니다.");
        }
    };
    if !minutes.is_finite() {
        // 입력은 유효하지만 너무 커서 계산이 넘친 경우 (예: distance_km=1e308)
        return detail(StatusCode::UNPROCESSABLE_ENTITY, "입력이 너무 커서 예상 시간을 계산할 수 없습니다.");
    }
    Json(json!({"minutes": minutes, "model": st.model})).into_response()
}

fn detail(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({"detail": msg}))).into_response()
}

fn method_not_allowed(allow: &'static str) -> Response {
    let mut resp = detail(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed");
    resp.headers_mut().insert(header::ALLOW, header::HeaderValue::from_static(allow));
    resp
}

/// FastAPI(pydantic) 422 응답의 detail 항목과 같은 모양.
fn field_error(kind: &str, loc: Value, msg: &str, input: Value) -> Value {
    json!({"type": kind, "loc": loc, "msg": msg, "input": input})
}

/// ge=0 위반. pydantic 은 ctx.ge 를 필드 타입대로 싣는다 (float 필드는 0.0, int 필드는 0).
fn ge_zero(loc: Value, input: &Value, ge: Value) -> Value {
    let mut e = field_error("greater_than_equal", loc, "Input should be greater than or equal to 0", input.clone());
    e["ctx"] = json!({"ge": ge});
    e
}

const NOT_OBJECT: &str = "Input should be a valid dictionary or object to extract fields from";

/// EtaRequest 검증 규칙을 그대로 따른다.
///   - distance_km: JSON 숫자(정수·소수)만, 유한, >= 0. 문자열·bool·null 은 거부.
///   - pending_orders: JSON 정수 표기만(2.0, 1e2 거부), >= 0. 아주 큰 정수도 받는다.
///   - 모르는 필드는 무시한다.
fn parse_eta_request(content_type: Option<&str>, raw: &[u8]) -> Result<(f64, f64), Vec<Value>> {
    if let Some(ct) = content_type {
        if !is_json_media_type(ct) {
            let input = Value::String(String::from_utf8_lossy(raw).into_owned());
            return Err(vec![field_error("model_attributes_type", json!(["body"]), NOT_OBJECT, input)]);
        }
    }
    if raw.trim_ascii().is_empty() {
        return Err(vec![field_error("missing", json!(["body"]), "Field required", Value::Null)]);
    }

    // Python json 처럼 NaN·Infinity 리터럴을 받아 들인 뒤 필드 검증에서 거부한다(깨진 JSON 이 아니라 필드 오류).
    let body: Value = match serde_json::from_slice(&mark_non_finite(raw)) {
        Ok(v) => v,
        Err(e) => {
            return Err(vec![field_error(
                "json_invalid",
                json!(["body", error_offset(raw, &e)]),
                "JSON decode error",
                json!({}),
            )]);
        }
    };
    let Value::Object(obj) = body else {
        return Err(vec![field_error("model_attributes_type", json!(["body"]), NOT_OBJECT, restore_non_finite(&body))]);
    };

    let distance = parse_distance(&obj);
    let orders = parse_orders(&obj);
    match (distance, orders) {
        (Ok(d), Ok(o)) => Ok((d, o)),
        (d, o) => Err([d.err(), o.err()].into_iter().flatten().collect()),
    }
}

fn parse_distance(obj: &Map<String, Value>) -> Result<f64, Value> {
    const NAME: &str = "distance_km";
    let loc = json!(["body", NAME]);
    let Some(v) = obj.get(NAME) else {
        return Err(field_error("missing", loc, "Field required", restore_non_finite(&Value::Object(obj.clone()))));
    };
    match v {
        Value::Number(n) => {
            let f: f64 = n.as_str().parse().unwrap_or(f64::INFINITY);
            if !f.is_finite() {
                // 1e400 처럼 float64 를 넘는 값. Python json 은 이를 inf 로 읽으므로 input 도 "inf" 다.
                return Err(field_error("finite_number", loc, "Input should be a finite number", json!("inf")));
            }
            if f < 0.0 {
                return Err(ge_zero(loc, v, json!(0.0)));
            }
            Ok(f)
        }
        Value::String(s) if non_finite_literal(s).is_some() => {
            let lit = non_finite_literal(s).unwrap_or_default();
            Err(field_error("finite_number", loc, "Input should be a finite number", json!(lit)))
        }
        _ => Err(field_error("float_type", loc, "Input should be a valid number", restore_non_finite(v))),
    }
}

fn parse_orders(obj: &Map<String, Value>) -> Result<f64, Value> {
    const NAME: &str = "pending_orders";
    let loc = json!(["body", NAME]);
    let Some(v) = obj.get(NAME) else {
        return Err(field_error("missing", loc, "Field required", restore_non_finite(&Value::Object(obj.clone()))));
    };
    let int_type = || field_error("int_type", loc.clone(), "Input should be a valid integer", restore_non_finite(v));
    let Value::Number(n) = v else {
        return Err(int_type());
    };
    let text = n.as_str();
    if text.contains(['.', 'e', 'E']) {
        return Err(int_type());
    }
    // 정수 표기만 남았다. 아주 큰 정수는 +Inf → 예측이 넘쳐 422.
    let f: f64 = text.parse().map_err(|_| int_type())?;
    if f < 0.0 {
        return Err(ge_zero(loc, v, json!(0)));
    }
    Ok(f + 0.0) // "-0" → 0
}

fn is_json_media_type(content_type: &str) -> bool {
    let mt = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let Some((kind, sub)) = mt.split_once('/') else {
        return false;
    };
    let token = |s: &str| !s.is_empty() && !s.contains(|c: char| c.is_whitespace() || c == '/');
    if !token(kind) || !token(sub) {
        return false;
    }
    mt == "application/json" || (kind == "application" && sub.ends_with("+json"))
}

/// serde_json 오류의 줄·열을 원래 본문의 바이트 위치로 바꾼다 (FastAPI 의 loc ["body", 위치] 처럼).
fn error_offset(raw: &[u8], e: &serde_json::Error) -> usize {
    let (line, column) = (e.line(), e.column());
    if line == 0 {
        return 0;
    }
    let line_start: usize = raw.split(|&b| b == b'\n').take(line - 1).map(|l| l.len() + 1).sum();
    (line_start + column.saturating_sub(1)).min(raw.len())
}

// 문자열 바깥의 NaN·Infinity·-Infinity 리터럴을 이 표식이 붙은 JSON 문자열로 바꾼다.
const NON_FINITE_MARK: &str = "\u{0}nonfinite:";
const NON_FINITE_TOKENS: [&str; 3] = ["-Infinity", "Infinity", "NaN"];

fn mark_non_finite(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let (mut in_string, mut escaped) = (false, false);
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_string = true;
            out.push(c);
            i += 1;
            continue;
        }
        if let Some(tok) = NON_FINITE_TOKENS.iter().find(|t| raw[i..].starts_with(t.as_bytes())) {
            out.extend_from_slice(format!("\"\\u0000nonfinite:{tok}\"").as_bytes());
            i += tok.len();
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// 표식이 붙은 문자열이면 Python str(float) 표기("nan", "inf", "-inf")를 돌려준다.
fn non_finite_literal(s: &str) -> Option<&'static str> {
    match s.strip_prefix(NON_FINITE_MARK)? {
        "NaN" => Some("nan"),
        "Infinity" => Some("inf"),
        "-Infinity" => Some("-inf"),
        _ => None,
    }
}

/// 오류 응답의 input 에 실을 값에서 표식을 원래 리터럴 문자열로 되돌린다.
fn restore_non_finite(v: &Value) -> Value {
    match v {
        Value::String(s) => match non_finite_literal(s) {
            Some(lit) => Value::String(lit.to_string()),
            None => v.clone(),
        },
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), restore_non_finite(x))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(restore_non_finite).collect()),
        _ => v.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    /// api/predictors/rule.py 와 같은 식. 핸들러 계약만 확인하려고 모델 대신 쓴다.
    /// float32 를 거치는 ONNX 모델처럼 float32 범위를 넘으면 +Inf 가 된다.
    fn rule_predict() -> Predict {
        Arc::new(|d, o| Ok(f64::from((10.0 + 3.0 * d + 2.0 * o) as f32)))
    }

    async fn call(method: &str, path: &str, body: &str, content_type: Option<&str>) -> (StatusCode, HeaderMap, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(ct) = content_type {
            req = req.header(header::CONTENT_TYPE, ct);
        }
        let resp = router(rule_predict(), "onnx")
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, headers, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn post_json(body: &str) -> (StatusCode, Value) {
        let (s, _, b) = call("POST", "/eta", body, Some("application/json")).await;
        (s, b)
    }

    /// /health 는 200 과 status=ok, 구현체 이름을 돌려준다.
    #[tokio::test]
    async fn health_ok() {
        let (s, _, b) = call("GET", "/health", "", None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(b, json!({"status": "ok", "model": "onnx"}));
    }

    /// /eta 는 정상·경계 입력과 모르는 필드·-0.0·아주 큰 정수에 200 과 예상 분, 구현체 이름을 돌려준다.
    #[tokio::test]
    async fn eta_known_inputs() {
        let cases: [(&str, f64); 8] = [
            (r#"{"distance_km": 0, "pending_orders": 0}"#, 10.0),
            (r#"{"distance_km": 2, "pending_orders": 3}"#, 22.0),
            (r#"{"distance_km": 1.5, "pending_orders": 0}"#, 14.5),
            (r#"{"distance_km": 1000, "pending_orders": 10000}"#, 23010.0),
            (r#"{"distance_km": 1, "pending_orders": 0, "x": 1}"#, 13.0),
            (r#"{"distance_km": -0.0, "pending_orders": 0}"#, 10.0),
            (r#"{"distance_km": 0, "pending_orders": -0}"#, 10.0),
            (r#"{"distance_km": 0, "pending_orders": 100000000000000000000000}"#, 2e23),
        ];
        for (body, want) in cases {
            let (s, b) = post_json(body).await;
            assert_eq!(s, StatusCode::OK, "{body}: {b}");
            let obj = b.as_object().unwrap();
            assert_eq!(obj.len(), 2, "{body}: {b}");
            assert_eq!(obj["model"], "onnx");
            let got: f64 = obj["minutes"].as_f64().unwrap();
            assert!((got - want).abs() <= 1e-6 * want.max(1.0), "{body}: minutes={got} want {want}");
        }
    }

    /// /eta 는 음수·숫자 아님·정수 아님·null·문자열 숫자·bool·필드 누락에 422 를 돌려준다.
    #[tokio::test]
    async fn eta_invalid_fields_return_422() {
        for body in [
            r#"{"distance_km": -1, "pending_orders": 0}"#,
            r#"{"distance_km": 0, "pending_orders": -1}"#,
            r#"{"distance_km": -1e-9, "pending_orders": 0}"#,
            r#"{"distance_km": "abc", "pending_orders": 0}"#,
            r#"{"distance_km": 1, "pending_orders": "many"}"#,
            r#"{"distance_km": 1, "pending_orders": 1.5}"#,
            r#"{"distance_km": 1, "pending_orders": 2.0}"#,
            r#"{"distance_km": 1, "pending_orders": 1e2}"#,
            r#"{"distance_km": "2", "pending_orders": 3}"#,
            r#"{"distance_km": 2, "pending_orders": "3"}"#,
            r#"{"distance_km": true, "pending_orders": 0}"#,
            r#"{"distance_km": 1, "pending_orders": false}"#,
            r#"{"distance_km": null, "pending_orders": 0}"#,
            r#"{"distance_km": 0, "pending_orders": null}"#,
            r#"{"distance_km": 0, "pending_orders": NaN}"#,
            r#"{"distance_km": 1}"#,
            r#"{"pending_orders": 1}"#,
            r#"{}"#,
        ] {
            let (s, b) = post_json(body).await;
            assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {b}");
            assert!(b["detail"].is_array(), "{body}: {b}");
        }
    }

    /// /eta 는 두 필드가 모두 틀리면 detail 에 두 오류를 함께 싣는다.
    #[tokio::test]
    async fn eta_reports_both_field_errors() {
        let (s, b) = post_json(r#"{"distance_km": -1, "pending_orders": "x"}"#).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
        let locs: Vec<&Value> = b["detail"].as_array().unwrap().iter().map(|e| &e["loc"][1]).collect();
        assert_eq!(locs, [&json!("distance_km"), &json!("pending_orders")]);
    }

    /// /eta 는 객체가 아닌 JSON·깨진 JSON·빈 본문·form 인코딩·잘못된 Content-Type 에 400·415 가 아니라 422 를 돌려준다.
    #[tokio::test]
    async fn eta_malformed_body_returns_422() {
        for (body, ct) in [
            ("[1, 2]", Some("application/json")),
            ("{bad", Some("application/json")),
            (r#"{"distance_km": 1, "pending_orders": 0} x"#, Some("application/json")),
            ("", Some("application/json")),
            ("", None),
            ("distance_km=1&pending_orders=0", Some("application/x-www-form-urlencoded")),
            (r#"{"distance_km": 1, "pending_orders": 0}"#, Some("text/plain")),
        ] {
            let (s, _, b) = call("POST", "/eta", body, ct).await;
            assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body:?} ({ct:?}): {b}");
        }
    }

    /// /eta 는 Content-Type 이 없거나 application/*+json·charset 이 붙어도 JSON 으로 받는다.
    #[tokio::test]
    async fn eta_accepts_json_media_type_variants() {
        for ct in [None, Some("application/json; charset=utf-8"), Some("application/vnd.api+json")] {
            let (s, _, b) = call("POST", "/eta", r#"{"distance_km": 2, "pending_orders": 3}"#, ct).await;
            assert_eq!(s, StatusCode::OK, "{ct:?}: {b}");
            assert_eq!(b["minutes"].as_f64(), Some(22.0));
        }
    }

    /// /eta 의 422 응답은 detail[].loc 로 문제 필드를 가리킨다 (NaN·Infinity 리터럴 포함).
    #[tokio::test]
    async fn eta_422_detail_points_to_field() {
        for body in [
            r#"{"distance_km": -1, "pending_orders": 0}"#,
            r#"{"distance_km": NaN, "pending_orders": 0}"#,
            r#"{"distance_km": Infinity, "pending_orders": 0}"#,
            r#"{"distance_km": -Infinity, "pending_orders": 0}"#,
            r#"{"distance_km": 1e400, "pending_orders": 0}"#,
        ] {
            let (s, b) = post_json(body).await;
            assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {b}");
            let found = b["detail"].as_array().unwrap().iter().any(|e| e["loc"] == json!(["body", "distance_km"]));
            assert!(found, "{body}: detail 에 [body, distance_km] 가 없다: {b}");
        }
    }

    /// 문자열 안의 "NaN" 은 리터럴로 바꾸지 않는다 (그냥 문자열이라 float_type).
    #[tokio::test]
    async fn eta_nan_inside_string_is_plain_string() {
        let (s, b) = post_json(r#"{"distance_km": "NaN", "pending_orders": 0}"#).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(b["detail"][0]["type"], "float_type");
        assert_eq!(b["detail"][0]["input"], "NaN");
    }

    /// /eta 는 계산이 넘칠 만큼 큰 거리(1e308)에 minutes=null 대신 422 와 문자열 detail 을 돌려준다.
    #[tokio::test]
    async fn eta_overflow_returns_422() {
        let (s, b) = post_json(r#"{"distance_km": 1e308, "pending_orders": 0}"#).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(b["detail"].is_string(), "{b}");
    }

    /// /eta 는 GET 에, /health 는 POST 에 405 와 Allow 헤더를 돌려주고, 모르는 경로는 404 다.
    #[tokio::test]
    async fn method_not_allowed_and_not_found() {
        let (s, h, b) = call("GET", "/eta", "", None).await;
        assert_eq!((s, h.get(header::ALLOW).unwrap().to_str().unwrap()), (StatusCode::METHOD_NOT_ALLOWED, "POST"));
        assert_eq!(b, json!({"detail": "Method Not Allowed"}));
        let (s, h, _) = call("POST", "/health", "", None).await;
        assert_eq!((s, h.get(header::ALLOW).unwrap().to_str().unwrap()), (StatusCode::METHOD_NOT_ALLOWED, "GET"));
        let (s, _, b) = call("GET", "/nope", "", None).await;
        assert_eq!((s, b), (StatusCode::NOT_FOUND, json!({"detail": "Not Found"})));
    }

    /// /eta 는 1MiB 를 넘는 본문에 413 을 돌려준다.
    #[tokio::test]
    async fn eta_body_too_large_returns_413() {
        let big = format!(r#"{{"distance_km": 1, "pending_orders": 0, "pad": "{}"}}"#, "x".repeat(MAX_BODY_BYTES));
        let (s, _, _) = call("POST", "/eta", &big, Some("application/json")).await;
        assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    }
}
