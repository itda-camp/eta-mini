import importlib
import math
import pkgutil

import pytest

import api.predictors

PREDICTOR_NAMES = sorted(m.name for m in pkgutil.iter_modules(api.predictors.__path__))


def test_at_least_one_predictor_exists():
    """api.predictors 에 구현체가 하나 이상 있다."""
    assert "rule" in PREDICTOR_NAMES


@pytest.mark.parametrize("name", PREDICTOR_NAMES)
@pytest.mark.parametrize(
    ("distance_km", "pending_orders"),
    [(0, 0), (0.1, 0), (2.5, 3), (1000, 0), (0, 10000)],
)
def test_predictor_returns_finite_non_negative_float(name, distance_km, pending_orders):
    """모든 구현체는 유효 입력에 대해 0 이상의 유한한 float 을 돌려준다."""
    predict = importlib.import_module(f"api.predictors.{name}").estimate_delivery_minutes
    minutes = predict(distance_km, pending_orders)
    assert isinstance(minutes, float)
    assert math.isfinite(minutes)
    assert minutes >= 0
