use std::sync::Mutex;

use ort::session::Session;
use ort::value::Tensor;

/// 입력 "X" [1, 2] float32 → 출력 "variable" [1, 1] float32 모델을 실행한다.
/// 정규화는 모델 안에 있으므로 원래 단위 값을 그대로 넣는다.
/// ort 의 Session::run 은 &mut self 라서 Mutex 로 감싼다 (작은 모델이라 한 번에 한 요청씩 돌려도 충분하다).
pub struct OnnxModel {
    session: Mutex<Session>,
}

impl OnnxModel {
    /// ORT_LIB 의 ONNX Runtime 공유 라이브러리를 열고 모델 세션을 만든다.
    pub fn load(lib_path: &str, model_path: &str) -> Result<Self, String> {
        let env = ort::init_from(lib_path)
            .map_err(|e| format!("ONNX Runtime 공유 라이브러리를 열지 못했다 ({lib_path}): {e}"))?
            .with_name("delivery-eta")
            .build()
            .map_err(|e| format!("ONNX Runtime 초기화 실패 ({lib_path}): {e}"))?;
        let session = Session::builder(&env)
            .and_then(|mut b| b.commit_from_file(model_path))
            .map_err(|e| format!("모델 세션 생성 실패 ({model_path}): {e}"))?;
        Ok(Self { session: Mutex::new(session) })
    }

    /// 배달 예상 시간(분)을 돌려준다. float32 로 넘치는 입력은 Inf/NaN 결과가 되고,
    /// 이는 핸들러가 422 로 바꾼다.
    pub fn predict(&self, distance_km: f64, pending_orders: f64) -> Result<f64, String> {
        let input = Tensor::from_array(([1usize, 2], vec![distance_km as f32, pending_orders as f32]))
            .map_err(|e| e.to_string())?;
        let mut session = self.session.lock().map_err(|_| "세션 잠금이 깨졌다".to_string())?;
        let outputs = session.run(ort::inputs!["X" => input]).map_err(|e| e.to_string())?;
        let (_, data) = outputs["variable"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        data.first().map(|&v| f64::from(v)).ok_or_else(|| "모델 출력이 비어 있다".to_string())
    }
}
