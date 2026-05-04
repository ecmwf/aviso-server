#!/usr/bin/env python3
# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Smoke tests for Aviso streaming behavior.

Run:
    python3 scripts/smoke_test.py

Prerequisites:
    pip install httpx

    Copy configuration/config.yaml.example to configuration/config.yaml,
    or point the server at it with AVISOSERVER_CONFIG_FILE.

    When auth is enabled (the example config default), start auth-o-tron first:
        ./scripts/auth-o-tron-docker.sh

Environment:
    BASE_URL=http://127.0.0.1:8000
    BACKEND=in_memory|jetstream
    NATS_URL=nats://localhost:4222
    TIMEOUT_SECONDS=8

    AUTH_ENABLED=true|false (default: true)
        Must match the server's auth.enabled setting.
        When false, auth headers are omitted and auth-specific tests are skipped.
    AUTH_ADMIN_USER=admin-user
    AUTH_ADMIN_PASS=admin-pass

Optional JetStream expectation checks:
    JETSTREAM_POLICY_STREAM_NAME=POLYGON
    EXPECT_MAX_MESSAGES=...
    EXPECT_MAX_BYTES=...
    EXPECT_MAX_MESSAGES_PER_SUBJECT=...
    EXPECT_COMPRESSION=s2|none|true|false

End-to-end ECPDS plugin tests (only run against a binary built with
`--features ecpds` and a real ECPDS configured in the YAML):

    ECPDS_ENABLED=true
        Set to enable the ECPDS smoke cases. Default off.
    ECPDS_EVENT_TYPE=ecpds_test
        Schema name on the server that has plugins: ["ecpds"].
        IMPORTANT: this schema is assumed to have `match_key` as its
        ONLY required identifier field. The smoke test sends a minimal
        request body and does not paper over additional required
        fields — if your schema has more, add a dedicated minimal
        ECPDS test schema instead of pointing this at your production
        dissemination schema. See the Getting Started doc for an
        example test schema.
    ECPDS_MATCH_KEY=destination
        Identifier field used as the destination key.
    ECPDS_ALLOWED_USER=alice
        ECPDS username known to be authorised for ECPDS_ALLOWED_DESTINATION.
    ECPDS_ALLOWED_PASS=alice-pass
        Password for that user (used by direct-mode auth-o-tron flow).
    ECPDS_ALLOWED_DESTINATION=CIP
        A destination value the allowed user is entitled to read.
    ECPDS_DENIED_DESTINATION=NOT-A-REAL-DEST
        A destination value the allowed user is NOT entitled to read.

These tests verify allow / deny on YOUR ECPDS by hitting the running
Aviso server with the supplied credentials. They do NOT need an
Aviso-side mock; they call the real upstream.
"""

from __future__ import annotations

import base64
import json
import os
import shutil
import subprocess
import sys
import time
from argparse import ArgumentParser
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Callable

try:
    import httpx
except ModuleNotFoundError as exc:
    print(
        "Missing required dependency 'httpx'. "
        "Install it with: python3 -m pip install httpx",
        file=sys.stderr,
    )
    raise SystemExit(1) from exc


DEFAULT_DATE = "20250706"
DEFAULT_TIME = "1200"
TEST_POLYGON = "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)"
OUTSIDE_POLYGON = "(10.0,10.0,10.2,10.0,10.2,10.2,10.0,10.2,10.0,10.0)"


def _basic_auth_header(username: str, password: str) -> str:
    credentials = base64.b64encode(f"{username}:{password}".encode()).decode()
    return f"Basic {credentials}"


@dataclass(frozen=True)
class Config:
    base_url: str = os.getenv("BASE_URL", "http://127.0.0.1:8000")
    backend: str = os.getenv("BACKEND", "in_memory")
    nats_url: str = os.getenv("NATS_URL", "nats://localhost:4222")
    timeout_seconds: int = int(os.getenv("TIMEOUT_SECONDS", "8"))
    policy_stream_name: str = os.getenv("JETSTREAM_POLICY_STREAM_NAME", "POLYGON")
    expect_max_messages: str = os.getenv("EXPECT_MAX_MESSAGES", "")
    expect_max_bytes: str = os.getenv("EXPECT_MAX_BYTES", "")
    expect_max_messages_per_subject: str = os.getenv("EXPECT_MAX_MESSAGES_PER_SUBJECT", "")
    expect_compression: str = os.getenv("EXPECT_COMPRESSION", "")
    auth_enabled: bool = os.getenv("AUTH_ENABLED", "true").strip().lower() in {"1", "true", "yes", "on"}
    auth_admin_user: str = os.getenv("AUTH_ADMIN_USER", "admin-user")
    auth_admin_pass: str = os.getenv("AUTH_ADMIN_PASS", "admin-pass")
    ecpds_enabled: bool = os.getenv("ECPDS_ENABLED", "false").strip().lower() in {"1", "true", "yes", "on"}
    ecpds_event_type: str = os.getenv("ECPDS_EVENT_TYPE", "ecpds_test")
    ecpds_match_key: str = os.getenv("ECPDS_MATCH_KEY", "destination")
    ecpds_allowed_user: str = os.getenv("ECPDS_ALLOWED_USER", "")
    ecpds_allowed_pass: str = os.getenv("ECPDS_ALLOWED_PASS", "")
    ecpds_allowed_destination: str = os.getenv("ECPDS_ALLOWED_DESTINATION", "")
    ecpds_denied_destination: str = os.getenv("ECPDS_DENIED_DESTINATION", "NOT-A-REAL-DEST")
    verbose: bool = False

    def admin_auth_headers(self) -> dict[str, str]:
        if not self.auth_enabled:
            return {}
        return {"Authorization": _basic_auth_header(self.auth_admin_user, self.auth_admin_pass)}

    def auth_headers_for(self, username: str, password: str) -> dict[str, str]:
        if not self.auth_enabled:
            return {}
        return {"Authorization": _basic_auth_header(username, password)}


@dataclass(frozen=True)
class SmokeCase:
    name: str
    func: Callable[[Config], None]


class SmokeFailure(RuntimeError):
    pass


def truncate_text(value: str, limit: int = 500) -> str:
    if len(value) <= limit:
        return value
    return f"{value[:limit]}...<truncated {len(value) - limit} chars>"


def pretty_json(value: object) -> str:
    return json.dumps(value, indent=2, sort_keys=True)


def pretty_json_text(value: str) -> str:
    try:
        parsed = json.loads(value)
    except json.JSONDecodeError:
        return value
    return json.dumps(parsed, indent=2, sort_keys=True)


def pretty_sse_chunk_text(chunk: str) -> str:
    lines = chunk.splitlines()
    pretty_lines: list[str] = []
    for line in lines:
        if line.startswith("data: "):
            raw = line[len("data: ") :]
            pretty = pretty_json_text(raw)
            if pretty == raw:
                pretty_lines.append(line)
                continue
            pretty_lines.append("data:")
            pretty_lines.extend(pretty.splitlines())
        else:
            pretty_lines.append(line)
    return "\n".join(pretty_lines)


def verbose_log(config: Config, message: str) -> None:
    if config.verbose:
        print(f"[VERBOSE] {message}")


def now_iso_utc() -> str:
    return datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")


def unique_token(prefix: str) -> str:
    return f"{prefix}-{time.time_ns()}"


def build_timeout(config: Config, *, read: float | None = None) -> httpx.Timeout:
    read_timeout = max(1.0, float(config.timeout_seconds)) if read is None else read
    return httpx.Timeout(
        connect=min(config.timeout_seconds, 5.0),
        read=read_timeout,
        write=min(config.timeout_seconds, 5.0),
        pool=min(config.timeout_seconds, 5.0),
    )


def publish_burst(action: Callable[[], None], *, count: int = 3, interval_seconds: float = 0.35) -> None:
    for _ in range(count):
        action()
        time.sleep(interval_seconds)


def request_json(
    config: Config,
    method: str,
    path: str,
    body: dict | None = None,
    *,
    headers: dict[str, str] | None = None,
) -> tuple[int, str]:
    timeout = build_timeout(config)
    request_headers = headers if headers is not None else {}
    try:
        with httpx.Client(base_url=config.base_url, timeout=timeout) as client:
            verbose_log(
                config,
                (
                    f"HTTP {method} {path} request=\n"
                    f"{truncate_text(pretty_json(body), 2000)}"
                    if body is not None
                    else f"HTTP {method} {path} request=<none>"
                ),
            )
            response = client.request(method, path, json=body, headers=request_headers)
    except httpx.HTTPError as exc:
        raise SmokeFailure(f"request failed ({method} {path}): {exc}") from exc
    verbose_log(
        config,
        (
            f"HTTP {method} {path} status={response.status_code} body=\n"
            f"{truncate_text(pretty_json_text(response.text), 2000)}"
        ),
    )
    return response.status_code, response.text


def post_notification(
    config: Config,
    *,
    event_type: str,
    identifier: dict[str, str],
    payload: object,
    headers: dict[str, str] | None = None,
) -> None:
    auth_headers = headers if headers is not None else config.admin_auth_headers()
    status, body = request_json(
        config,
        "POST",
        "/api/v1/notification",
        {
            "event_type": event_type,
            "identifier": identifier,
            "payload": payload,
        },
        headers=auth_headers,
    )
    if status != 200:
        raise SmokeFailure(f"notification failed with status {status}: {body}")


def post_test_polygon_notification(
    config: Config,
    *,
    note: str,
    polygon: str,
    date_value: str = DEFAULT_DATE,
    time_value: str = DEFAULT_TIME,
) -> None:
    post_notification(
        config,
        event_type="test_polygon",
        identifier={
            "date": date_value,
            "time": time_value,
            "polygon": polygon,
        },
        payload={"note": note},
    )


def post_mars_notification(
    config: Config,
    *,
    note: str,
    stream_value: str,
    domain: str = "g",
    step: int = 1,
) -> None:
    post_notification(
        config,
        event_type="mars",
        identifier={
            "class": "od",
            "expver": "0001",
            "domain": domain,
            "date": DEFAULT_DATE,
            "time": DEFAULT_TIME,
            "stream": stream_value,
            "step": str(step),
        },
        payload=note,
    )


def post_dissemination_notification(config: Config, *, note: str, target_value: str) -> None:
    post_notification(
        config,
        event_type="dissemination",
        identifier={
            "destination": "FOO",
            "target": target_value,
            "class": "od",
            "expver": "0001",
            "domain": "g",
            "date": DEFAULT_DATE,
            "time": DEFAULT_TIME,
            "stream": "enfo",
            "step": "1",
        },
        payload={"note": note},
    )


def replay_body(config: Config, body: dict, *, headers: dict[str, str] | None = None) -> str:
    timeout = build_timeout(config)
    auth_headers = headers if headers is not None else config.admin_auth_headers()
    chunks: list[str] = []
    try:
        with httpx.Client(base_url=config.base_url, timeout=timeout, headers=auth_headers) as client:
            verbose_log(
                config,
                "HTTP POST /api/v1/replay stream request=\n"
                + truncate_text(pretty_json(body), 2000),
            )
            with client.stream("POST", "/api/v1/replay", json=body) as response:
                for text in response.iter_text():
                    chunks.append(text)
                    verbose_log(
                        config,
                        "SSE replay chunk=\n"
                        + truncate_text(pretty_sse_chunk_text(text), 2000),
                    )
                if response.status_code >= 400:
                    verbose_log(
                        config,
                        f"HTTP POST /api/v1/replay stream status={response.status_code}",
                    )
                    return "".join(chunks) or response.text
    except httpx.HTTPError as exc:
        raise SmokeFailure(f"replay request failed: {exc}") from exc
    verbose_log(config, "HTTP POST /api/v1/replay stream status=200")
    return "".join(chunks)


def capture_watch_output(
    config: Config,
    *,
    body: dict,
    after_start: Callable[[], None],
    publish_trigger: str,
    startup_deadline_seconds: float = 5.0,
    post_publish_capture_seconds: float = 4.0,
    headers: dict[str, str] | None = None,
) -> str:
    timeout = build_timeout(config, read=0.5)
    auth_headers = headers if headers is not None else config.admin_auth_headers()
    output_parts: list[str] = []
    accumulated_output = ""
    startup_deadline = time.monotonic() + startup_deadline_seconds
    after_start_done = False

    try:
        with httpx.Client(base_url=config.base_url, timeout=timeout, headers=auth_headers) as client:
            verbose_log(
                config,
                "HTTP POST /api/v1/watch stream request=\n"
                + truncate_text(pretty_json(body), 2000),
            )
            with client.stream("POST", "/api/v1/watch", json=body) as response:
                if response.status_code != 200:
                    verbose_log(
                        config,
                        "HTTP POST /api/v1/watch stream "
                        f"status={response.status_code} body=\n"
                        f"{truncate_text(pretty_json_text(response.text), 2000)}",
                    )
                    raise SmokeFailure(
                        f"watch request failed with status {response.status_code}: {response.text}"
                    )
                verbose_log(config, "HTTP POST /api/v1/watch stream status=200")
                chunks = response.iter_text()
                while time.monotonic() < startup_deadline:
                    try:
                        chunk = next(chunks)
                        output_parts.append(chunk)
                        accumulated_output += chunk
                        verbose_log(
                            config,
                            "SSE watch chunk=\n"
                            + truncate_text(pretty_sse_chunk_text(chunk), 2000),
                        )
                        if not after_start_done and publish_trigger in accumulated_output:
                            verbose_log(
                                config,
                                f"trigger matched ({publish_trigger}); publishing live notifications",
                            )
                            after_start()
                            after_start_done = True
                            break
                    except StopIteration:
                        return "".join(output_parts)
                    except httpx.ReadTimeout:
                        continue

                if not after_start_done:
                    after_start()
                    after_start_done = True

                post_publish_deadline = time.monotonic() + post_publish_capture_seconds
                while time.monotonic() < post_publish_deadline:
                    try:
                        chunk = next(chunks)
                        output_parts.append(chunk)
                        verbose_log(
                            config,
                            "SSE watch chunk=\n"
                            + truncate_text(pretty_sse_chunk_text(chunk), 2000),
                        )
                    except StopIteration:
                        break
                    except httpx.ReadTimeout:
                        continue
    except httpx.HTTPError as exc:
        raise SmokeFailure(f"watch request failed: {exc}") from exc

    if not after_start_done:
        try:
            verbose_log(
                config,
                "trigger not observed before startup deadline; publishing live notifications anyway",
            )
            after_start()
        except Exception as exc:  # pragma: no cover - best-effort cleanup path
            raise SmokeFailure(
                f"failed to send live event after opening watch stream: {exc}"
            ) from exc

    return "".join(output_parts)


def assert_contains(haystack: str, needle: str, context: str) -> None:
    if needle not in haystack:
        snippet = haystack[:800].replace("\n", "\\n")
        raise SmokeFailure(
            f"expected '{needle}' in {context}; stream_snippet='{snippet}'"
        )


def assert_not_contains(haystack: str, needle: str, context: str) -> None:
    if needle in haystack:
        raise SmokeFailure(f"did not expect '{needle}' in {context}")


def test_health(config: Config) -> None:
    status, _ = request_json(config, "GET", "/health")
    if status != 200:
        raise SmokeFailure(f"expected 200, got {status}")


def test_replay_requires_start_parameter(config: Config) -> None:
    status, _ = request_json(
        config,
        "POST",
        "/api/v1/replay",
        {
            "event_type": "test_polygon",
            "identifier": {"time": DEFAULT_TIME, "polygon": TEST_POLYGON},
        },
    )
    if status != 400:
        raise SmokeFailure(f"expected 400, got {status}")


def test_watch_live_only(config: Config) -> None:
    historical_note = unique_token("smoke-watch-historical")
    live_note = unique_token("smoke-watch-live")
    post_test_polygon_notification(config, note=historical_note, polygon=TEST_POLYGON)

    def publish_live_burst() -> None:
        publish_burst(
            lambda: post_test_polygon_notification(
                config, note=live_note, polygon=TEST_POLYGON
            )
        )

    output = capture_watch_output(
        config,
        body={
            "event_type": "test_polygon",
            "identifier": {"time": DEFAULT_TIME, "polygon": TEST_POLYGON},
        },
        after_start=publish_live_burst,
        publish_trigger='"type":"connection_established"',
    )
    assert_contains(output, live_note, "watch stream output")
    assert_not_contains(output, historical_note, "watch stream output")


def test_replay_from_id(config: Config) -> None:
    old_note = unique_token("smoke-replay-id-old")
    new_note = unique_token("smoke-replay-id-new")
    post_test_polygon_notification(config, note=old_note, polygon=TEST_POLYGON)
    post_test_polygon_notification(config, note=new_note, polygon=TEST_POLYGON)

    output = replay_body(
        config,
        {
            "event_type": "test_polygon",
            "identifier": {"time": DEFAULT_TIME, "polygon": TEST_POLYGON},
            "from_id": "1",
        },
    )
    assert_contains(output, new_note, "replay output")


def test_replay_from_date(config: Config) -> None:
    old_note = unique_token("smoke-replay-date-old")
    new_note = unique_token("smoke-replay-date-new")
    post_test_polygon_notification(config, note=old_note, polygon=TEST_POLYGON)
    time.sleep(1.0)
    boundary = now_iso_utc()
    time.sleep(1.0)
    post_test_polygon_notification(config, note=new_note, polygon=TEST_POLYGON)

    output = replay_body(
        config,
        {
            "event_type": "test_polygon",
            "identifier": {"time": DEFAULT_TIME, "polygon": TEST_POLYGON},
            "from_date": boundary,
        },
    )
    assert_contains(output, new_note, "replay output")
    assert_not_contains(output, old_note, "replay output")


def test_replay_point_filter(config: Config) -> None:
    inside_note = unique_token("smoke-replay-point-inside")
    outside_note = unique_token("smoke-replay-point-outside")
    boundary = now_iso_utc()
    time.sleep(1.0)

    # Different dates ensure distinct subjects when duplicates are disabled per subject.
    post_test_polygon_notification(
        config, note=inside_note, polygon=TEST_POLYGON, date_value="20250706"
    )
    post_test_polygon_notification(
        config, note=outside_note, polygon=OUTSIDE_POLYGON, date_value="20250707"
    )

    output = replay_body(
        config,
        {
            "event_type": "test_polygon",
            "identifier": {"time": DEFAULT_TIME, "point": "52.55,13.5"},
            "from_date": boundary,
        },
    )
    assert_contains(output, inside_note, "point-filter replay output")
    assert_not_contains(output, outside_note, "point-filter replay output")


def test_mars_replay_with_dot_identifier(config: Config) -> None:
    stream_value = unique_token("ens.member")
    first_note = unique_token("smoke-mars-replay-first")
    second_note = unique_token("smoke-mars-replay-second")
    post_mars_notification(config, note=first_note, stream_value=stream_value)
    post_mars_notification(config, note=second_note, stream_value=stream_value)

    output = replay_body(
        config,
        {
            "event_type": "mars",
            "identifier": {
                "class": "od",
                "expver": "0001",
                "domain": "g",
                "date": DEFAULT_DATE,
                "time": DEFAULT_TIME,
                "stream": stream_value,
                "step": "1",
            },
            "from_id": "1",
        },
    )
    assert_contains(output, stream_value, "mars replay output")


def test_dissemination_watch_from_date(config: Config) -> None:
    target_value = unique_token("target.v1")
    historical_note = unique_token("smoke-diss-watch-old")
    live_note = unique_token("smoke-diss-watch-live")

    post_dissemination_notification(config, note=historical_note, target_value=target_value)
    time.sleep(1.0)
    boundary = now_iso_utc()

    def publish_live_burst() -> None:
        publish_burst(
            lambda: post_dissemination_notification(
                config, note=live_note, target_value=target_value
            )
        )

    output = capture_watch_output(
        config,
        body={
            "event_type": "dissemination",
            "identifier": {
                "destination": "FOO",
                "target": target_value,
                "class": "od",
                "expver": "0001",
                "domain": "g",
                "date": DEFAULT_DATE,
                "time": DEFAULT_TIME,
                "stream": "enfo",
                "step": "1",
            },
            "from_date": boundary,
        },
        after_start=publish_live_burst,
        publish_trigger='"type":"replay_completed"',
    )
    assert_contains(output, live_note, "dissemination watch output")
    assert_not_contains(output, historical_note, "dissemination watch output")


def test_mars_replay_with_int_predicate(config: Config) -> None:
    stream_value = unique_token("ens.int-filter")
    low_note = unique_token("smoke-mars-int-low")
    high_note = unique_token("smoke-mars-int-high")

    post_mars_notification(
        config, note=low_note, stream_value=stream_value, domain="g", step=2
    )
    post_mars_notification(
        config, note=high_note, stream_value=stream_value, domain="g", step=6
    )

    output = replay_body(
        config,
        {
            "event_type": "mars",
            "identifier": {
                "class": "od",
                "expver": "0001",
                "domain": "g",
                "date": DEFAULT_DATE,
                "time": DEFAULT_TIME,
                "stream": stream_value,
                "step": {"gte": 4},
            },
            "from_id": "1",
        },
    )
    assert_contains(output, high_note, "mars int-predicate replay output")
    assert_not_contains(output, low_note, "mars int-predicate replay output")


def test_mars_replay_with_enum_in_predicate(config: Config) -> None:
    stream_value = unique_token("ens.enum-filter")
    include_note = unique_token("smoke-mars-enum-include")
    exclude_note = unique_token("smoke-mars-enum-exclude")

    post_mars_notification(
        config, note=include_note, stream_value=stream_value, domain="g", step=1
    )
    post_mars_notification(
        config, note=exclude_note, stream_value=stream_value, domain="z", step=1
    )

    output = replay_body(
        config,
        {
            "event_type": "mars",
            "identifier": {
                "class": "od",
                "expver": "0001",
                "domain": {"in": ["g", "a"]},
                "date": DEFAULT_DATE,
                "time": DEFAULT_TIME,
                "stream": stream_value,
                "step": "1",
            },
            "from_id": "1",
        },
    )
    assert_contains(output, include_note, "mars enum-predicate replay output")
    assert_not_contains(output, exclude_note, "mars enum-predicate replay output")


def test_auth_public_stream_no_credentials(config: Config) -> None:
    """test_polygon has no auth block — requests without credentials should succeed."""
    if not config.auth_enabled:
        print("[INFO] skipping auth test because AUTH_ENABLED=false")
        return
    post_notification(
        config,
        event_type="test_polygon",
        identifier={"date": DEFAULT_DATE, "time": DEFAULT_TIME, "polygon": TEST_POLYGON},
        payload={"note": "auth-public-no-creds"},
        headers={},
    )


def test_auth_mars_unauthenticated_rejected(config: Config) -> None:
    """mars has auth.required=true — unauthenticated requests should get 401."""
    if not config.auth_enabled:
        print("[INFO] skipping auth test because AUTH_ENABLED=false")
        return
    status, _ = request_json(
        config,
        "POST",
        "/api/v1/notification",
        {
            "event_type": "mars",
            "identifier": {
                "class": "od",
                "expver": "0001",
                "domain": "g",
                "date": DEFAULT_DATE,
                "time": DEFAULT_TIME,
                "stream": "oper",
                "step": "1",
            },
            "payload": "auth-test",
        },
        headers={},
    )
    if status != 401:
        raise SmokeFailure(f"expected 401 for unauthenticated mars notify, got {status}")


def test_auth_mars_reader_can_read(config: Config) -> None:
    """mars read_roles: localrealm: ["*"] — reader-user should be able to replay."""
    if not config.auth_enabled:
        print("[INFO] skipping auth test because AUTH_ENABLED=false")
        return
    reader_headers = config.auth_headers_for("reader-user", "reader-pass")
    stream_value = unique_token("auth-reader-read")
    seed_note = unique_token("auth-reader-seed")
    post_mars_notification(config, note=seed_note, stream_value=stream_value)
    output = replay_body(
        config,
        {
            "event_type": "mars",
            "identifier": {
                "class": "od",
                "expver": "0001",
                "domain": "g",
                "date": DEFAULT_DATE,
                "time": DEFAULT_TIME,
                "stream": stream_value,
                "step": "1",
            },
            "from_id": "1",
        },
        headers=reader_headers,
    )
    assert_contains(output, seed_note, "mars replay as reader-user")


def test_auth_mars_reader_cannot_write(config: Config) -> None:
    """mars write_roles: localrealm: ["producer"] — reader-user should get 403 on notify."""
    if not config.auth_enabled:
        print("[INFO] skipping auth test because AUTH_ENABLED=false")
        return
    reader_headers = config.auth_headers_for("reader-user", "reader-pass")
    status, _ = request_json(
        config,
        "POST",
        "/api/v1/notification",
        {
            "event_type": "mars",
            "identifier": {
                "class": "od",
                "expver": "0001",
                "domain": "g",
                "date": DEFAULT_DATE,
                "time": DEFAULT_TIME,
                "stream": "oper",
                "step": "1",
            },
            "payload": "auth-test-write-blocked",
        },
        headers=reader_headers,
    )
    if status != 403:
        raise SmokeFailure(f"expected 403 for reader writing to mars, got {status}")


def test_auth_mars_producer_can_write(config: Config) -> None:
    """mars write_roles: localrealm: ["producer"] — producer-user should succeed."""
    if not config.auth_enabled:
        print("[INFO] skipping auth test because AUTH_ENABLED=false")
        return
    producer_headers = config.auth_headers_for("producer-user", "producer-pass")
    status, _ = request_json(
        config,
        "POST",
        "/api/v1/notification",
        {
            "event_type": "mars",
            "identifier": {
                "class": "od",
                "expver": "0001",
                "domain": "g",
                "date": DEFAULT_DATE,
                "time": DEFAULT_TIME,
                "stream": "oper",
                "step": "1",
            },
            "payload": "auth-test-producer-write",
        },
        headers=producer_headers,
    )
    if status != 200:
        raise SmokeFailure(f"expected 200 for producer writing to mars, got {status}")


def test_auth_dissemination_reader_can_read(config: Config) -> None:
    """dissemination has no read_roles — any authenticated user can replay."""
    if not config.auth_enabled:
        print("[INFO] skipping auth test because AUTH_ENABLED=false")
        return
    reader_headers = config.auth_headers_for("reader-user", "reader-pass")
    target_value = unique_token("auth-diss-reader")
    seed_note = unique_token("auth-diss-seed")
    post_dissemination_notification(config, note=seed_note, target_value=target_value)
    output = replay_body(
        config,
        {
            "event_type": "dissemination",
            "identifier": {
                "destination": "FOO",
                "target": target_value,
                "class": "od",
                "expver": "0001",
                "domain": "g",
                "date": DEFAULT_DATE,
                "time": DEFAULT_TIME,
                "stream": "enfo",
                "step": "1",
            },
            "from_id": "1",
        },
        headers=reader_headers,
    )
    assert_contains(output, seed_note, "dissemination replay as reader-user")


def test_auth_dissemination_reader_cannot_write(config: Config) -> None:
    """dissemination has no write_roles — only admins can write. reader-user should get 403."""
    if not config.auth_enabled:
        print("[INFO] skipping auth test because AUTH_ENABLED=false")
        return
    reader_headers = config.auth_headers_for("reader-user", "reader-pass")
    status, _ = request_json(
        config,
        "POST",
        "/api/v1/notification",
        {
            "event_type": "dissemination",
            "identifier": {
                "destination": "FOO",
                "target": "bar",
                "class": "od",
                "expver": "0001",
                "domain": "g",
                "date": DEFAULT_DATE,
                "time": DEFAULT_TIME,
                "stream": "enfo",
                "step": "1",
            },
            "payload": {"note": "auth-test-diss-blocked"},
        },
        headers=reader_headers,
    )
    if status != 403:
        raise SmokeFailure(f"expected 403 for reader writing to dissemination, got {status}")


def _ecpds_skip_reason(config: Config) -> str | None:
    if not config.ecpds_enabled:
        return "ECPDS_ENABLED is not set"
    if not config.auth_enabled:
        return "ECPDS plugin requires AUTH_ENABLED=true"
    missing = [
        name
        for name, value in [
            ("ECPDS_ALLOWED_USER", config.ecpds_allowed_user),
            ("ECPDS_ALLOWED_PASS", config.ecpds_allowed_pass),
            ("ECPDS_ALLOWED_DESTINATION", config.ecpds_allowed_destination),
        ]
        if not value
    ]
    if missing:
        return f"required env not set: {', '.join(missing)}"
    return None


def _ecpds_identifier(config: Config, destination: str) -> dict:
    return {config.ecpds_match_key: destination}


def _ecpds_failure_hint_for_400() -> str:
    return (
        "got 400 from the schema validator before the request reached the ECPDS plugin. "
        "The smoke test sends a minimal identifier ({match_key: destination}) and does not "
        "populate any other fields. If your ECPDS_EVENT_TYPE schema has additional "
        "required identifier fields, add a dedicated minimal ECPDS test schema as shown in "
        "the Getting Started doc instead of pointing the smoke test at your production schema."
    )


def _ecpds_post_status(
    config: Config, path: str, body: dict, headers: dict[str, str]
) -> tuple[int, str]:
    """POST and return (status, response_text), reading at most a small
    chunk of any SSE body. Used by the ECPDS smoke cases which only
    care about the HTTP status of the watch endpoint, not the stream
    payload."""
    timeout = build_timeout(config, read=2.0)
    try:
        with httpx.Client(
            base_url=config.base_url, timeout=timeout, headers=headers
        ) as client:
            with client.stream("POST", path, json=body) as response:
                first_chunk = ""
                if response.status_code == 200:
                    for chunk in response.iter_text():
                        first_chunk = chunk
                        break
                else:
                    first_chunk = response.read().decode("utf-8", errors="replace")
                return response.status_code, first_chunk
    except httpx.HTTPError as exc:
        raise SmokeFailure(f"request failed: {exc}") from exc


def test_ecpds_allowed_destination_returns_200(config: Config) -> None:
    """Allowed user reading an entitled destination must get a 200 watch stream."""
    skip = _ecpds_skip_reason(config)
    if skip:
        print(f"[INFO] skipping ECPDS smoke test ({skip})")
        return

    headers = config.auth_headers_for(
        config.ecpds_allowed_user, config.ecpds_allowed_pass
    )
    body = {
        "event_type": config.ecpds_event_type,
        "identifier": _ecpds_identifier(config, config.ecpds_allowed_destination),
    }

    status, response = _ecpds_post_status(config, "/api/v1/watch", body, headers)
    if status == 400:
        raise SmokeFailure(_ecpds_failure_hint_for_400() + f" response: {truncate_text(response)}")
    if status != 200:
        raise SmokeFailure(
            f"expected 200 for allowed user + allowed destination, got {status}; "
            f"response: {truncate_text(response)}"
        )


def test_ecpds_denied_destination_returns_403(config: Config) -> None:
    """Allowed user reading an unentitled destination must get a 403."""
    skip = _ecpds_skip_reason(config)
    if skip:
        print(f"[INFO] skipping ECPDS smoke test ({skip})")
        return

    headers = config.auth_headers_for(
        config.ecpds_allowed_user, config.ecpds_allowed_pass
    )
    body = {
        "event_type": config.ecpds_event_type,
        "identifier": _ecpds_identifier(config, config.ecpds_denied_destination),
    }

    status, response = _ecpds_post_status(config, "/api/v1/watch", body, headers)
    if status == 400:
        raise SmokeFailure(_ecpds_failure_hint_for_400() + f" response: {truncate_text(response)}")
    if status != 403:
        raise SmokeFailure(
            f"expected 403 for allowed user + DENIED destination, got {status}; "
            f"response: {truncate_text(response)}"
        )


def test_ecpds_notify_unaffected(config: Config) -> None:
    """NOTIFY on an ECPDS-protected stream must succeed (200) for the
    admin user. The plugin is read-only, so it must not gate writes —
    a 503 here would mean it incorrectly tried to consult ECPDS on a
    write, and a 403/401 would mean admin auth is broken (a config
    issue with `AUTH_ADMIN_USER`/`AUTH_ADMIN_PASS`, not the plugin).
    Either way, anything other than 2xx is a failure."""
    skip = _ecpds_skip_reason(config)
    if skip:
        print(f"[INFO] skipping ECPDS smoke test ({skip})")
        return

    body = {
        "event_type": config.ecpds_event_type,
        "identifier": _ecpds_identifier(config, "any-value-not-checked"),
        "payload": {"note": "ecpds-notify-bypass-smoke"},
    }
    status, response = request_json(
        config,
        "POST",
        "/api/v1/notification",
        body,
        headers=config.admin_auth_headers(),
    )
    if not (200 <= status < 300):
        raise SmokeFailure(
            f"expected 2xx on notify for an ECPDS-protected stream, got {status}; "
            f"a 503 means the plugin incorrectly ran on a write, a 401/403 "
            f"means AUTH_ADMIN_USER/AUTH_ADMIN_PASS need to match your auth-o-tron "
            f"config. response: {truncate_text(response)}"
        )


def expected_compression_value(raw: str) -> str:
    normalized = raw.strip().lower()
    if normalized in {"true", "s2"}:
        return "s2"
    if normalized in {"false", "none"}:
        return "none"
    return normalized


def test_jetstream_policy_visibility(config: Config) -> None:
    if config.backend != "jetstream":
        print(f"[INFO] skipping policy inspection because BACKEND={config.backend}")
        return
    if shutil.which("nats") is None:
        print("[INFO] skipping policy inspection because nats CLI is not installed")
        return

    result = subprocess.run(
        [
            "nats",
            "--server",
            config.nats_url,
            "stream",
            "info",
            config.policy_stream_name,
            "--json",
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        print(
            f"[INFO] skipping policy inspection because stream "
            f"{config.policy_stream_name} is unavailable"
        )
        return
    verbose_log(
        config,
        "nats stream info raw=\n" + truncate_text(pretty_json_text(result.stdout), 2000),
    )

    try:
        info = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise SmokeFailure(f"invalid JSON from nats stream info: {exc}") from exc

    config_obj = info.get("config", {})
    required_fields = [
        "max_msgs",
        "max_bytes",
        "max_age",
        "max_msgs_per_subject",
        "compression",
    ]
    missing = [field for field in required_fields if field not in config_obj]
    if missing:
        raise SmokeFailure(f"missing JetStream policy fields: {', '.join(missing)}")

    if config.expect_max_messages:
        actual = str(config_obj.get("max_msgs"))
        if actual != config.expect_max_messages:
            raise SmokeFailure(
                f"max_msgs mismatch: expected {config.expect_max_messages}, got {actual}"
            )
    if config.expect_max_bytes:
        actual = str(config_obj.get("max_bytes"))
        if actual != config.expect_max_bytes:
            raise SmokeFailure(
                f"max_bytes mismatch: expected {config.expect_max_bytes}, got {actual}"
            )
    if config.expect_max_messages_per_subject:
        actual = str(config_obj.get("max_msgs_per_subject"))
        if actual != config.expect_max_messages_per_subject:
            raise SmokeFailure(
                "max_msgs_per_subject mismatch: expected "
                f"{config.expect_max_messages_per_subject}, got {actual}"
            )
    if config.expect_compression:
        actual = str(config_obj.get("compression", "")).lower()
        expected = expected_compression_value(config.expect_compression)
        if actual != expected:
            raise SmokeFailure(f"compression mismatch: expected {expected}, got {actual}")


def run_case(case: SmokeCase, config: Config) -> tuple[bool, str]:
    try:
        case.func(config)
        return True, ""
    except SmokeFailure as exc:
        return False, str(exc)
    except Exception as exc:  # pragma: no cover - defensive branch for operator visibility
        return False, f"unexpected error: {exc}"


def main() -> int:
    parser = ArgumentParser(description="Run Aviso smoke tests")
    parser.add_argument("-v", "--verbose", action="store_true", help="Enable verbose request/response logging")
    args = parser.parse_args()

    env_verbose = os.getenv("SMOKE_VERBOSE", "").strip().lower() in {"1", "true", "yes", "on"}
    config = Config(verbose=args.verbose or env_verbose)

    cases = [
        SmokeCase("health endpoint returns 200", test_health),
        SmokeCase(
            "replay requires from_id or from_date",
            test_replay_requires_start_parameter,
        ),
        SmokeCase("watch without replay params is live-only", test_watch_live_only),
        SmokeCase("replay with from_id returns historical stream", test_replay_from_id),
        SmokeCase("replay with from_date excludes older messages", test_replay_from_date),
        SmokeCase("replay with point returns only containing polygons", test_replay_point_filter),
        SmokeCase(
            "mars replay with from_id works for dot-containing identifier values",
            test_mars_replay_with_dot_identifier,
        ),
        SmokeCase(
            "diss watch with from_date excludes old and includes live for dot-containing identifier values",
            test_dissemination_watch_from_date,
        ),
        SmokeCase(
            "mars replay supports integer predicates under identifier",
            test_mars_replay_with_int_predicate,
        ),
        SmokeCase(
            "mars replay supports enum in-predicate under identifier",
            test_mars_replay_with_enum_in_predicate,
        ),
        SmokeCase(
            "auth: public stream accepts unauthenticated requests",
            test_auth_public_stream_no_credentials,
        ),
        SmokeCase(
            "auth: mars rejects unauthenticated requests",
            test_auth_mars_unauthenticated_rejected,
        ),
        SmokeCase(
            "auth: mars reader can replay (wildcard read_roles)",
            test_auth_mars_reader_can_read,
        ),
        SmokeCase(
            "auth: mars reader cannot write (missing producer role)",
            test_auth_mars_reader_cannot_write,
        ),
        SmokeCase(
            "auth: mars producer can write",
            test_auth_mars_producer_can_write,
        ),
        SmokeCase(
            "auth: dissemination reader can replay (no read_roles restriction)",
            test_auth_dissemination_reader_can_read,
        ),
        SmokeCase(
            "auth: dissemination reader cannot write (admin-only)",
            test_auth_dissemination_reader_cannot_write,
        ),
        SmokeCase(
            "jetstream stream policy is inspectable (and optionally matches expected values)",
            test_jetstream_policy_visibility,
        ),
        SmokeCase(
            "ecpds: allowed user + entitled destination -> 200",
            test_ecpds_allowed_destination_returns_200,
        ),
        SmokeCase(
            "ecpds: allowed user + DENIED destination -> 403",
            test_ecpds_denied_destination_returns_403,
        ),
        SmokeCase(
            "ecpds: notify on ECPDS-protected stream is not gated",
            test_ecpds_notify_unaffected,
        ),
    ]

    print(
        f"[INFO] running smoke tests against {config.base_url} "
        f"(backend={config.backend}, auth={config.auth_enabled}, timeout={config.timeout_seconds}s)"
    )
    passed = 0
    failed = 0
    for case in cases:
        ok, reason = run_case(case, config)
        if ok:
            passed += 1
            print(f"[PASS] {case.name}")
        else:
            failed += 1
            print(f"[FAIL] {case.name}")
            if reason:
                print(f"       reason: {reason}")

    print(f"\n[INFO] smoke summary: pass={passed} fail={failed}")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
