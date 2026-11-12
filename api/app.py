import importlib
import math
import os
from fastapi import FastAPI, HTTPException, Request
from fastapi.exception_handlers import request_validation_exception_handler
from fastapi.exceptions import RequestValidationError
from pydantic import BaseModel, Field

# 어떤 방식의 API를 쓸 것인지를 외부에서 환경변수를 통해 주입
MODEL_IMPL = os.getenv("MODEL_IMPL", "rule")
estimate_delivery_minutes = importlib.import_module(f"api.predictors.{MODEL_IMPL}").estimate_delivery_minutes

class EtaRequest(BaseModel):
    """배달 예상 시간 요청"""
    # 모델이 더 많은 정보를 원하면 여기에 필드를 추가
    # 아래 ge=0 와 같은 조건들을 유효성 검사 룰을 지정 (validation rule)
    # strict: "2" 같은 문자열 숫자, true/false, 정수 자리의 2.0 을 받지 않는다.
    # allow_inf_nan=False: NaN·Infinity 거리를 받지 않는다.
    # 모르는 필드는 무시한다(모델이 필드를 늘릴 때 클라이언트가 먼저 보내도 깨지지 않게).
    distance_km: float = Field(ge=0, strict=True, allow_inf_nan=False, description="가게에서 배달지까지 거리(km)")
    pending_orders: int = Field(ge=0, strict=True, description="지금 밀려 있는 주문 수")

class EtaResponse(BaseModel):
    """배달 예상 시간 응답"""
    minutes: float
    model: str  # 어떤 구현체가 낸 예측인지 (장애 추적용)

app = FastAPI(title="delivery-eta")


def _json_safe(value):
    """NaN·Infinity 처럼 JSON 으로 쓸 수 없는 float 을 문자열로 바꾼다."""
    if isinstance(value, float) and not math.isfinite(value):
        return str(value)
    if isinstance(value, dict):
        return {k: _json_safe(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [_json_safe(v) for v in value]
    return value


@app.exception_handler(RequestValidationError)
async def validation_exception_handler(request: Request, exc: RequestValidationError):
    """422 응답에 그대로 실리는 입력값이 NaN·Infinity 여도 500 이 나지 않게 한다."""
    safe = RequestValidationError(_json_safe(list(exc.errors())), body=exc.body)
    return await request_validation_exception_handler(request, safe)

@app.get("/health")
def health() -> dict[str, str]:
    """서비스 상태와 현재 구현체 이름을 돌려준다."""
    return {"status": "ok", "model": MODEL_IMPL}

@app.post("/eta", response_model=EtaResponse)
def eta(req: EtaRequest) -> EtaResponse:
    """배달 예상 시간을 계산"""
    minutes = estimate_delivery_minutes(req.distance_km, req.pending_orders)
    if not math.isfinite(minutes):
        # 입력은 유효하지만 너무 커서 계산이 넘친 경우 (예: distance_km=1e308)
        raise HTTPException(status_code=422, detail="입력이 너무 커서 예상 시간을 계산할 수 없습니다.")
    return EtaResponse(minutes=minutes, model=MODEL_IMPL)
