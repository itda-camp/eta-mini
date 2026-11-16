"""Python API 와 다른 언어 서비스(Go, Rust)의 예측이 같은지 비교한다.

기준은 학습한 원본 모델(MODEL_IMPL=sklearn)로 띄운 Python API 다.
같은 입력을 기준과 비교 대상 모두에 POST /eta 로 보내고, 대상마다 최대 오차를 잰다.
최대 오차가 TOLERANCE 분 미만이면 PASS. 하나라도 FAIL 이면 exit 1 (CI 가 멈추도록).

서버가 떠 있어야 하므로 pytest 테스트가 아니다. 표준 라이브러리만 쓴다.
    PY_URL=http://127.0.0.1:8080 \\
    TARGETS=go=http://127.0.0.1:8081,rust=http://127.0.0.1:8082 \\
    python -m training.check_parity
"""

import json
import os
import random
import sys
import urllib.error
import urllib.request

TOLERANCE = 1e-4  # 분
TIMEOUT = 5.0     # 초. 요청 하나가 이보다 오래 걸리면 실패로 본다
PY_URL = os.getenv("PY_URL", "http://127.0.0.1:8080").rstrip("/")
TARGETS_ENV = os.getenv("TARGETS", "go=http://127.0.0.1:8081,rust=http://127.0.0.1:8082")


def parse_targets(spec: str) -> dict[str, str]:
    """'go=http://...,rust=http://...' 를 {이름: URL} 로 바꾼다."""
    targets = {}
    for item in spec.split(","):
        item = item.strip()
        if not item:
            continue
        name, sep, url = item.partition("=")
        if not sep or not name.strip() or not url.strip():
            raise ValueError(f"TARGETS 형식이 잘못됐다: {item!r} (이름=URL 이어야 한다)")
        targets[name.strip()] = url.strip().rstrip("/")
    if not targets:
        raise ValueError("TARGETS 가 비어 있다")
    return targets


def make_inputs() -> list[tuple[float, int]]:
    """고정 입력 3건 + seed 0 무작위 20건 (거리 0.5~8km, 주문 0~10건)."""
    fixed = [(3.0, 2), (0.0, 0), (8.0, 10)]
    rng = random.Random(0)
    randoms = [(rng.uniform(0.5, 8.0), rng.randint(0, 10)) for _ in range(20)]
    return fixed + randoms


def http_json(url: str, payload: dict | None = None) -> dict:
    """GET(payload 없음) 또는 POST(JSON) 요청을 보내고 JSON 응답을 돌려준다."""
    data = None if payload is None else json.dumps(payload).encode()
    req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
        return json.loads(resp.read())


def predict(base_url: str, distance_km: float, pending_orders: int) -> float:
    """서비스의 POST /eta 로 예상 시간(분)을 받는다."""
    body = http_json(f"{base_url}/eta", {"distance_km": distance_km, "pending_orders": pending_orders})
    return float(body["minutes"])


def main() -> int:
    try:
        targets = parse_targets(TARGETS_ENV)
    except ValueError as e:
        print(f"FAIL {e}")
        return 1
    inputs = make_inputs()

    # 기준 서비스가 정말 원본 모델로 떠 있는지 확인한다. 아니면 비교 자체가 의미 없다.
    try:
        model = http_json(f"{PY_URL}/health").get("model")
        if model != "sklearn":
            print(f"FAIL [python] 기준 서비스가 MODEL_IMPL=sklearn 이 아니다 (model={model!r}, {PY_URL})")
            return 1
        expected = [predict(PY_URL, d, o) for d, o in inputs]
    except (urllib.error.URLError, OSError, ValueError, KeyError) as e:
        print(f"FAIL [python] 기준 서비스 {PY_URL} 에서 예측을 받지 못했다: {e}")
        return 1

    all_ok = True
    for name, url in targets.items():
        try:
            got = [predict(url, d, o) for d, o in inputs]
        except (urllib.error.URLError, OSError, ValueError, KeyError) as e:
            print(f"FAIL [{name}] {url} 에서 예측을 받지 못했다: {e}")
            all_ok = False
            continue
        diffs = [abs(g - w) for g, w in zip(got, expected)]
        worst = max(range(len(diffs)), key=diffs.__getitem__)
        ok = all(x < TOLERANCE for x in diffs)  # NaN 이 섞이면 비교가 False 라 FAIL 이 된다
        all_ok &= ok
        d, o = inputs[worst]
        print(("PASS " if ok else "FAIL ")
              + f"[{name}] {len(inputs)}건 최대 오차 {diffs[worst]:.2e}분 (기준 < {TOLERANCE:g})"
              + f" — 최악 입력 ({d:.3f}km, {o}건): python {expected[worst]:.6f} vs {name} {got[worst]:.6f}")

    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
