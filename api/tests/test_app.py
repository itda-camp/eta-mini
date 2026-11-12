import pytest
from fastapi.testclient import TestClient

from api.app import app

client = TestClient(app)


def test_health():
    """/health 는 200 과 status=ok, 구현체 이름을 돌려준다."""
    res = client.get("/health")
    assert res.status_code == 200
    assert res.json() == {"status": "ok", "model": "rule"}


@pytest.mark.parametrize(
    ("payload", "expected"),
    [
        ({"distance_km": 0, "pending_orders": 0}, 10.0),
        ({"distance_km": 2, "pending_orders": 3}, 22.0),
        ({"distance_km": 1.5, "pending_orders": 0}, 14.5),
        ({"distance_km": 1000, "pending_orders": 10000}, 23010.0),
    ],
)
def test_eta_known_inputs(payload, expected):
    """/eta 는 정상·경계 입력에 대해 200 과 예상 분, 구현체 이름을 돌려준다."""
    res = client.post("/eta", json=payload)
    assert res.status_code == 200
    body = res.json()
    assert set(body) == {"minutes", "model"}
    assert body["minutes"] == pytest.approx(expected)
    assert body["model"] == "rule"


@pytest.mark.parametrize(
    "payload",
    [
        {"distance_km": -1, "pending_orders": 0},
        {"distance_km": 0, "pending_orders": -1},
        {"distance_km": "abc", "pending_orders": 0},
        {"distance_km": 1, "pending_orders": "many"},
        {"distance_km": 1, "pending_orders": 1.5},
        {"distance_km": 1},
        {"pending_orders": 1},
        {},
    ],
)
def test_eta_invalid_input_returns_422(payload):
    """/eta 는 음수·숫자 아님·정수 아님·필드 누락 입력에 422 를 돌려준다."""
    res = client.post("/eta", json=payload)
    assert res.status_code == 422
