from __future__ import annotations

from typing import Any

from fastapi.testclient import TestClient

from flowai_harness.studio import create_studio_app
from flowai_harness.studio.server import STUDIO_AUTH_HEADER

STUDIO_TEST_AUTH_TOKEN = "test-studio-token"


def create_studio_test_client(app: Any, **kwargs: Any) -> TestClient:
    client = TestClient(
        create_studio_app(app, auth_token=STUDIO_TEST_AUTH_TOKEN, **kwargs)
    )
    client.headers.update({STUDIO_AUTH_HEADER: STUDIO_TEST_AUTH_TOKEN})
    return client
