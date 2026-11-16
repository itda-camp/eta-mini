//! delivery-eta 의 Rust 구현. api/app.py 와 같은 HTTP 계약(POST /eta, GET /health)을 따르고,
//! 예측은 ONNX 모델(model_sklearn.onnx)을 ONNX Runtime 으로 실행한다.
//!
//! 환경 변수:
//!   PORT        듣는 포트 (기본 8080)
//!   MODEL_PATH  ONNX 모델 경로 (기본 artifacts/model_sklearn.onnx)
//!   ORT_LIB     libonnxruntime 공유 라이브러리 경로 (기본 libonnxruntime.so)

mod eta;
mod model;

use std::sync::Arc;

/// /health 와 /eta 응답의 model 값 (Python 의 MODEL_IMPL=onnx, Go 구현과 같은 이름).
const MODEL_NAME: &str = "onnx";

fn getenv(key: &str, fallback: &str) -> String {
    std::env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| fallback.to_string())
}

#[tokio::main]
async fn main() {
    let ort_lib = getenv("ORT_LIB", "libonnxruntime.so");
    let model_path = getenv("MODEL_PATH", "artifacts/model_sklearn.onnx");
    let model = match model::OnnxModel::load(&ort_lib, &model_path) {
        Ok(m) => Arc::new(m),
        Err(e) => {
            eprintln!("모델을 불러오지 못했다: {e}");
            std::process::exit(1);
        }
    };
    let predict: eta::Predict = Arc::new(move |d, o| model.predict(d, o));

    let addr = format!("0.0.0.0:{}", getenv("PORT", "8080"));
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{addr} 에서 들을 수 없다: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("delivery-eta (rust, model={MODEL_NAME}) listening on {addr}");
    if let Err(e) = axum::serve(listener, eta::router(predict, MODEL_NAME))
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        eprintln!("서버 오류: {e}");
        std::process::exit(1);
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}
