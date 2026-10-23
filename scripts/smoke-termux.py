#!/usr/bin/env python3
"""Exercise a native panel + real Minecraft JVM in a disposable directory.

Requires Python 3, a built panel with web/dist embedded, and a discoverable
Android-compatible Java 21. Downloads the vanilla server from Mojang. Explicit
--accept-eula is required. No account credentials or Playit tunnels are created
outside this disposable local panel. Leaves logs/data for failure diagnosis.
"""
import argparse
import http.cookiejar
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import time
import urllib.error
import urllib.request


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--accept-eula", action="store_true")
    parser.add_argument("--version", default="1.21.8")
    args = parser.parse_args()
    if not args.accept_eula:
        parser.error("Read https://aka.ms/MinecraftEULA and pass --accept-eula to run Minecraft")
    binary = args.binary.resolve(strict=True)
    work = args.work_dir.resolve()
    work.mkdir(parents=True, exist_ok=True)
    if any(work.iterdir()):
        parser.error("--work-dir must be empty to keep existing panel data untouched")
    panel_port, game_port = free_port(), free_port()
    while game_port == panel_port:
        game_port = free_port()
    base = f"http://127.0.0.1:{panel_port}"
    cookies = http.cookiejar.CookieJar()
    opener = urllib.request.build_opener(
        urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(cookies)
    )
    environment = dict(os.environ)
    # No claiming/account requests during a local server lifecycle test.
    environment["MCPANEL_NO_TRAY"] = "1"
    with (work / "panel.log").open("w") as log:
        panel = subprocess.Popen([
            str(binary), "--data-dir", str(work / "data"), "--bind",
            f"127.0.0.1:{panel_port}", "--allow-unsandboxed-servers",
            "--playit-mode", "external",
        ], env=environment, stdout=log, stderr=subprocess.STDOUT)

        def request(path, body=None, method=None):
            headers = {"Origin": base}
            csrf = next((c.value for c in cookies if c.name == "mcpanel_csrf"), None)
            if csrf:
                headers["x-csrf-token"] = csrf
            if body is not None:
                headers["Content-Type"] = "application/json"
            req = urllib.request.Request(
                base + path, data=json.dumps(body).encode() if body is not None else None,
                headers=headers, method=method,
            )
            try:
                with opener.open(req, timeout=600) as response:
                    raw = response.read()
                    if "application/json" in response.headers.get("Content-Type", ""):
                        return json.loads(raw)
                    return raw.decode()
            except urllib.error.HTTPError as error:
                raise RuntimeError(f"{req.method} {path}: {error.code} {error.read().decode()}") from error

        def wait_status(server, wanted, previous_pid=None):
            deadline = time.monotonic() + 240
            while time.monotonic() < deadline:
                view = request(server)
                if view["status"] == wanted and (previous_pid is None or view.get("pid") != previous_pid):
                    return view
                if view["status"] == "crashed":
                    raise RuntimeError(request(server + "/logs"))
                time.sleep(1)
            raise RuntimeError(f"timed out waiting for {wanted}: {request(server + '/logs')}")

        try:
            for _ in range(60):
                if panel.poll() is not None:
                    raise RuntimeError("panel exited; see panel.log")
                try:
                    request("/api/setup/status")
                    break
                except (OSError, RuntimeError):
                    time.sleep(1)
            else:
                raise RuntimeError("panel did not start")
            assert "assets/index-" in request("/"), "frontend is not embedded"
            password = secrets.token_urlsafe(24)
            credentials = {"username": "termux-smoke", "password": password}
            request("/api/setup", {**credentials, "confirm": password})
            request("/api/auth/login", credentials)
            javas = request("/api/catalog/javas")
            assert any(java["major"] == 21 for java in javas), javas
            print("PASS embedded frontend, setup/login, Java 21 discovery", flush=True)
            created = request("/api/servers", {
                "name": "Termux smoke", "core": "vanilla", "version": args.version,
                "java_major": 21, "memory": {"min_mb": 256, "max_mb": 1024},
                "port": game_port, "eula_accepted": True,
            })
            server = "/api/servers/" + created["id"]
            request(server + "/prepare", {})
            # Keep world generation small on a phone and listen on loopback.
            props = request(server + "/files/read?path=server.properties")["content"]
            props += "view-distance=2\nsimulation-distance=2\nmax-players=2\nserver-ip=127.0.0.1\n"
            request(server + "/files", {"path": "server.properties", "content": props}, "PUT")
            request(server + "/power", {"action": "start"})
            online = wait_status(server, "online")
            assert online["pid"]
            host = request("/api/system")
            assert host["memory_total_mb"] > 0
            assert online["metrics"]["memory_mb"] > 0
            assert any("Done (" in line["line"] for line in request(server + "/logs"))
            request(server + "/command", {"command": "list"})
            print("PASS provisioning, real Minecraft online, console command", flush=True)
            request(server + "/power", {"action": "restart"})
            wait_status(server, "online", previous_pid=online["pid"])
            request(server + "/power", {"action": "stop"})
            wait_status(server, "offline")
            print("PASS restart and graceful stop", flush=True)
            request(server + "/files", {"path": "smoke.txt", "content": "before backup"}, "PUT")
            backup = request(server + "/backups", {"note": "Termux smoke"})
            request(server + "/files", {"path": "smoke.txt", "content": "after backup"}, "PUT")
            request(server + f"/backups/{backup['id']}/restore", {})
            assert request(server + "/files/read?path=smoke.txt")["content"] == "before backup"
            print("PASS file editing and backup/restore", flush=True)
            request(server + "/power", {"action": "start"})
            final_online = wait_status(server, "online")
            panel.terminate()
            assert panel.wait(timeout=90) == 0, "panel did not shut down cleanly"
            try:
                os.kill(final_online["pid"], 0)
            except ProcessLookupError:
                pass
            else:
                raise RuntimeError("JVM survived panel shutdown")
            print("PASS panel shutdown with a running JVM", flush=True)
        finally:
            if panel.poll() is None:
                panel.terminate()
                try:
                    panel.wait(timeout=90)
                except subprocess.TimeoutExpired:
                    panel.kill()
                    panel.wait()
            print(f"Smoke logs and data: {work}", flush=True)


if __name__ == "__main__":
    main()
