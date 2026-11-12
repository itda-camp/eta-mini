import importlib.util

import pytest
from fastapi.testclient import TestClient


def _load_app_module():
    """sys.modules 를 건드리지 않고 api.app 을 새로 실행해 돌려준다."""
    spec = importlib.util.find_spec("api.app")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_model_impl_defaults_to_rule(monkeypatch):
    """MODEL_IMPL 이 없으면 rule 구현체를 쓰고 /health 가 그 이름을 돌려준다."""
    monkeypatch.delenv("MODEL_IMPL", raising=False)
    module = _load_app_module()
    res = TestClient(module.app).get("/health")
    assert res.json()["model"] == "rule"


def test_model_impl_env_selects_implementation(monkeypatch):
    """MODEL_IMPL=rule 이면 /eta 응답의 model 이 rule 이다."""
    monkeypatch.setenv("MODEL_IMPL", "rule")
    module = _load_app_module()
    res = TestClient(module.app).post("/eta", json={"distance_km": 0, "pending_orders": 0})
    assert res.json() == {"minutes": 10.0, "model": "rule"}


def test_unknown_model_impl_fails_at_startup(monkeypatch):
    """없는 구현체 이름이면 앱을 불러오는 시점에 ModuleNotFoundError 로 실패한다."""
    monkeypatch.setenv("MODEL_IMPL", "does_not_exist")
    with pytest.raises(ModuleNotFoundError):
        _load_app_module()
