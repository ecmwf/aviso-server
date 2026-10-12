#!/usr/bin/env python3
# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Drive aviso-server's /mcp endpoint with the official MCP Python SDK.

Run through `cargo test --test mcp_transport -- --ignored`, which starts a
server and a local key set and passes these environment variables:

- MCP_URL: the /mcp URL to call
- MCP_TOKEN: a valid bearer token
- MCP_RESOURCE: the expected `resource` of the metadata document

Requires the `mcp` package (tested with 1.26). Exits non-zero on the first
failed check.
"""

import asyncio
import json
import os
from urllib.parse import urlsplit

import httpx
from mcp import ClientSession
from mcp.client.auth.utils import (
    extract_resource_metadata_from_www_auth,
    extract_scope_from_www_auth,
)
from mcp.client.streamable_http import streamablehttp_client
from mcp.shared.auth import ProtectedResourceMetadata

URL = os.environ["MCP_URL"]
TOKEN = os.environ["MCP_TOKEN"]
RESOURCE = os.environ["MCP_RESOURCE"]
EXPECTED_TOOLS = {
    "aviso_health",
    "aviso_list_event_types",
    "aviso_describe_event_type",
    "aviso_replay_notifications",
    "aviso_check_data_available",
}


def check(condition, message):
    if not condition:
        raise SystemExit(f"interop check failed: {message}")


async def discovery():
    """An unauthenticated call must lead a client to the metadata."""
    async with httpx.AsyncClient() as http:
        response = await http.post(
            URL,
            json={"jsonrpc": "2.0", "id": 1, "method": "ping"},
            headers={"Accept": "application/json, text/event-stream"},
        )
        check(response.status_code == 401, f"status {response.status_code}")
        metadata_url = extract_resource_metadata_from_www_auth(response)
        check(metadata_url is not None, "no resource_metadata in challenge")
        check(extract_scope_from_www_auth(response) == "aviso:read", "scope")
        # The advertised URL names the public host; fetch the same path here.
        local = URL.rsplit("/mcp", 1)[0] + urlsplit(metadata_url).path
        document = (await http.get(local)).json()
        metadata = ProtectedResourceMetadata.model_validate(document)
        check(str(metadata.resource) == RESOURCE, f"resource {metadata.resource}")


async def version_probe():
    """Unknown client revisions get the newest supported one."""
    async with httpx.AsyncClient() as http:
        response = await http.post(
            URL,
            json={
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2099-01-01",
                    "capabilities": {},
                    "clientInfo": {"name": "probe", "version": "0"},
                },
            },
            headers={
                "Accept": "application/json, text/event-stream",
                "Authorization": f"Bearer {TOKEN}",
            },
        )
        check(response.status_code == 200, f"status {response.status_code}")
        version = response.json()["result"]["protocolVersion"]
        check(version == "2025-11-25", f"negotiated {version}")
        check("mcp-session-id" not in response.headers, "session id sent")


async def session():
    headers = {"Authorization": f"Bearer {TOKEN}"}
    async with streamablehttp_client(URL, headers=headers) as (read, write, _):
        async with ClientSession(read, write) as client:
            init = await client.initialize()
            check(init.serverInfo.name == "aviso-server", "server name")
            await client.send_ping()

            tools = (await client.list_tools()).tools
            check({tool.name for tool in tools} == EXPECTED_TOOLS, "tool names")
            check(all(tool.outputSchema for tool in tools), "output schemas")

            health = await client.call_tool("aviso_health", {})
            check(not health.isError, "health failed")
            check(health.structuredContent["backend_connected"], "backend down")

            types = await client.call_tool("aviso_list_event_types", {})
            check("interop" in types.structuredContent["event_types"], "types")

            replay = await client.call_tool(
                "aviso_replay_notifications",
                {
                    "event_type": "interop",
                    "identifier": {"class": "od", "number": {"gte": 0}},
                },
            )
            check(not replay.isError, replay.content)
            check(replay.structuredContent["count"] == 2, "replay count")
            mirrored = json.loads(replay.content[0].text)
            check(mirrored == replay.structuredContent, "text mirror")

            missing = await client.call_tool(
                "aviso_replay_notifications", {"event_type": "interop"}
            )
            check(missing.isError, "bad arguments must be a tool error")


async def main():
    await discovery()
    await version_probe()
    await session()
    print("interop ok")


if __name__ == "__main__":
    asyncio.run(main())
