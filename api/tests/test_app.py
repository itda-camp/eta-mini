import pytest
from fastapi.testclient import TestClient

import api.app as app_module
from api.app import app

client = TestClient(app)


def test_health():
    """/health 는 200 과 status=ok, 현재 구현체 이름을 돌려준다."""
    res = client.get("/health")
    assert res.status_code == 200
    assert res.json() == {"status": "ok", "model": app_module.MODEL_IMPL}


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
    assert body["model"] == app_module.MODEL_IMPL


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


@pytest.mark.parametrize(
    "payload",
    [
        {"distance_km": None, "pending_orders": 0},
        {"distance_km": 0, "pending_orders": None},
        {"distance_km": -1e-9, "pending_orders": 0},
    ],
)
def test_eta_null_or_just_below_zero_returns_422(payload):
    """/eta 는 null 값과 0 바로 아래 음수에 422 를 돌려준다."""
    res = client.post("/eta", json=payload)
    assert res.status_code == 422


@pytest.mark.parametrize(
    ("content", "content_type"),
    [
        ("[1, 2]", "application/json"),
        ("{bad", "application/json"),
        ("distance_km=1&pending_orders=0", "application/x-www-form-urlencoded"),
    ],
)
def test_eta_malformed_body_returns_422(content, content_type):
    """/eta 는 객체가 아닌 JSON·깨진 JSON·form 인코딩 body 에 422 를 돌려준다."""
    res = client.post("/eta", content=content, headers={"content-type": content_type})
    assert res.status_code == 422


def test_eta_get_method_not_allowed():
    """/eta 는 GET 요청에 405 를 돌려준다."""
    assert client.get("/eta").status_code == 405


def test_eta_422_detail_points_to_field():
    """/eta 의 422 응답은 detail[].loc 로 문제 필드를 가리킨다."""
    res = client.post("/eta", json={"distance_km": -1, "pending_orders": 0})
    assert res.status_code == 422
    locs = [err["loc"] for err in res.json()["detail"]]
    assert ["body", "distance_km"] in locs


@pytest.mark.parametrize(
    "payload",
    [
        {"distance_km": "2", "pending_orders": 3},  # 문자열 숫자
        {"distance_km": 2, "pending_orders": "3"},
        {"distance_km": True, "pending_orders": 0},  # bool
        {"distance_km": 1, "pending_orders": False},
        {"distance_km": 1, "pending_orders": 2.0},  # 정수 자리의 2.0
    ],
)
def test_eta_rejects_string_bool_and_float_count(payload):
    """/eta 는 문자열 숫자·bool·정수 자리의 소수 표기를 받지 않고 422 를 돌려준다."""
    res = client.post("/eta", json=payload)
    assert res.status_code == 422


@pytest.mark.parametrize(
    ("payload", "expected"),
    [
        ({"distance_km": 1, "pending_orders": 0, "x": 1}, 13.0),  # 모르는 필드는 무시
        ({"distance_km": -0.0, "pending_orders": 0}, 10.0),  # -0.0 은 0 으로 받는다
        ({"distance_km": 0, "pending_orders": 10**23}, 2e23),  # 아주 큰 정수
    ],
)
def test_eta_accepts_extra_field_negative_zero_and_huge_int(payload, expected):
    """/eta 는 모르는 필드를 무시하고 -0.0·아주 큰 정수를 받아 200 을 돌려준다."""
    res = client.post("/eta", json=payload)
    assert res.status_code == 200
    assert res.json()["minutes"] == pytest.approx(expected)


@pytest.mark.parametrize("literal", ["NaN", "Infinity", "-Infinity"])
def test_eta_non_finite_distance_returns_422(literal):
    """/eta 는 NaN·Infinity 거리에 500 이 아니라 JSON 본문을 가진 422 를 돌려준다."""
    c = TestClient(app, raise_server_exceptions=False)
    res = c.post(
        "/eta",
        content=f'{{"distance_km": {literal}, "pending_orders": 0}}',
        headers={"content-type": "application/json"},
    )
    assert res.status_code == 422
    locs = [err["loc"] for err in res.json()["detail"]]
    assert ["body", "distance_km"] in locs


def test_eta_overflowing_distance_returns_422():
    """/eta 는 계산이 넘칠 만큼 큰 거리(1e308)에 minutes=null 대신 422 를 돌려준다."""
    res = client.post("/eta", json={"distance_km": 1e308, "pending_orders": 0})
    assert res.status_code == 422
