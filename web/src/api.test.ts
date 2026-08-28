import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, api, token } from "./api";

/** Reply with `body` as JSON. */
function jsonResponse(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function mockFetch(...responses: Response[]) {
  const fetchMock = vi.fn();
  for (const response of responses) fetchMock.mockResolvedValueOnce(response);
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

describe("authentication", () => {
  it("stores the token on login and sends it afterwards", async () => {
    const fetchMock = mockFetch(
      jsonResponse({ token: "abc123", user: { username: "admin", admin: true } }),
      jsonResponse([]),
    );

    await api.login("admin", "hunter2hunter2");
    expect(token()).toBe("abc123");

    await api.servers();
    const headers = fetchMock.mock.calls[1][1].headers as Headers;
    expect(headers.get("Authorization")).toBe("Bearer abc123");
  });

  it("drops the session and announces it on a 401", async () => {
    localStorage.setItem("mcpanel.token", "stale");
    mockFetch(jsonResponse({ error: "unauthorized" }, 401));

    const loggedOut = vi.fn();
    window.addEventListener("mcpanel:logout", loggedOut);

    await expect(api.servers()).rejects.toBeInstanceOf(ApiError);

    // Without both of these a dead session leaves the UI pretending to work.
    expect(token()).toBeNull();
    expect(loggedOut).toHaveBeenCalled();
  });

  it("surfaces the server's error message rather than a status code", async () => {
    mockFetch(jsonResponse({ error: "port 25565 is already assigned" }, 400));

    await expect(api.createServer({ name: "x" })).rejects.toThrow(
      "port 25565 is already assigned",
    );
  });
});

describe("playit", () => {
  beforeEach(() => {
    localStorage.setItem("mcpanel.token", "session-token");
  });

  it("uses the authenticated claim and server tunnel endpoints", async () => {
    const fetchMock = mockFetch(
      jsonResponse({ claim_url: "https://playit.gg/claim/abc" }),
      jsonResponse({ state: "provisioning", binding: null, tunnel: null, message: null }),
    );

    await api.playitClaim();
    await api.attachPlayit("server-1");

    expect(fetchMock.mock.calls[0][0]).toBe("/api/playit/claim");
    expect(fetchMock.mock.calls[0][1].method).toBe("POST");
    expect((fetchMock.mock.calls[0][1].headers as Headers).get("Authorization")).toBe(
      "Bearer session-token",
    );
    expect(fetchMock.mock.calls[1][0]).toBe("/api/servers/server-1/playit");
    expect(JSON.parse(fetchMock.mock.calls[1][1].body)).toEqual({});
  });

  it("escapes tunnel ids before putting them in a delete path", async () => {
    const fetchMock = mockFetch(jsonResponse({ ok: true }));

    await api.deletePlayitTunnel("tunnel/with spaces");

    expect(fetchMock.mock.calls[0][0]).toBe("/api/playit/tunnels/tunnel%2Fwith%20spaces");
  });
});

describe("downloads", () => {
  beforeEach(() => {
    localStorage.setItem("mcpanel.token", "session-token");
  });

  it("fetches a ticket and never puts the session token in the URL", async () => {
    const fetchMock = mockFetch(jsonResponse({ ticket: "tkt-123" }));
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});

    await api.download("srv-1", "server.properties");

    const [ticketUrl, ticketInit] = fetchMock.mock.calls[0];
    expect(ticketUrl).toContain("/files/ticket");
    expect(ticketInit.method).toBe("POST");
    // The ticket request authenticates by header, as every API call does.
    expect((ticketInit.headers as Headers).get("Authorization")).toBe(
      "Bearer session-token",
    );

    const navigated = click.mock.instances[0] as unknown as HTMLAnchorElement;
    expect(navigated.href).toContain("ticket=tkt-123");
    // The whole point: a session credential must not reach history or a log.
    expect(navigated.href).not.toContain("session-token");
    expect(navigated.href).not.toContain("token=");
  });

  it("uses a ticket for backups too", async () => {
    mockFetch(jsonResponse({ ticket: "tkt-backup" }));
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});

    await api.downloadBackup("srv-1", "20260819-160422");

    const navigated = click.mock.instances[0] as unknown as HTMLAnchorElement;
    expect(navigated.href).toContain("ticket=tkt-backup");
    expect(navigated.href).not.toContain("session-token");
  });
});

describe("uploads", () => {
  it("signs the operator out on a 401, like every other call", async () => {
    localStorage.setItem("mcpanel.token", "stale");
    mockFetch(new Response("", { status: 401 }));

    const loggedOut = vi.fn();
    window.addEventListener("mcpanel:logout", loggedOut);

    // FormData cannot go through the JSON helper, so this handling is repeated
    // rather than inherited — which is exactly why it needs its own test.
    await expect(
      api.upload("srv-1", "", [new File(["x"], "a.txt")]),
    ).rejects.toBeInstanceOf(ApiError);

    expect(token()).toBeNull();
    expect(loggedOut).toHaveBeenCalled();
  });
});
