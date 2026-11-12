import pytest

from api.predictors.rule import estimate_delivery_minutes


@pytest.mark.parametrize(
    ("distance_km", "pending_orders", "expected"),
    [
        (0, 0, 10.0),
        (1, 0, 13.0),
        (0, 1, 12.0),
        (2.5, 3, 23.5),
        (100, 1000, 2310.0),
        (0.1, 0, 10.3),
        (1000, 0, 3010.0),
        (0, 10000, 20010.0),
    ],
)
def test_known_inputs(distance_km, pending_orders, expected):
    """알려진 입력에 대해 10 + 3*거리 + 2*대기주문 값을 돌려준다."""
    assert estimate_delivery_minutes(distance_km, pending_orders) == pytest.approx(expected)


@pytest.mark.parametrize(("distance_km", "pending_orders"), [(0, 0), (1, 1), (1.5, 2)])
def test_returns_float(distance_km, pending_orders):
    """정수·소수 입력 모두 float 을 돌려준다."""
    assert isinstance(estimate_delivery_minutes(distance_km, pending_orders), float)
