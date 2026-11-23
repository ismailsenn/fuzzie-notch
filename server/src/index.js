// Fuzzie relay: friends, presence, pokes and listening parties.
//
// One Durable Object ("hub") keeps every account in SQLite and every open
// connection as a hibernatable WebSocket, so an idle relay costs nothing.
//
// HTTP
//   POST /register  {name}          -> {code, key}   (new account)
//   GET  /ws        Authorization: Bearer CODE:KEY  (WebSocket upgrade)
//
// Client -> relay (JSON text frames; "ping" is answered with "pong")
//   {t:"hello", name, skin, acc}
//   {t:"np", np:{title, artist, playing, pos, dur} | null}
//   {t:"add", code} {t:"accept", code} {t:"decline", code} {t:"remove", code}
//   {t:"poke", to}
//   {t:"join", to} {t:"leave", to}       (listening along with a friend)
//
// Relay -> client
//   {t:"state", now, me:{code, name}, friends:[{code, name, skin, acc, online, np}],
//    incoming:[{code, name}], outgoing:[{code, name}]}
//   {t:"poke", from, name, skin, acc}
//   {t:"join", from, name} {t:"leave", from, name}
//   {t:"error", msg}

import { DurableObject } from "cloudflare:workers";

const MAX_FRIENDS = 50;
const MAX_PENDING = 50;
const MAX_FRAME = 2048;
const POKE_GAP_MS = 2000;
// No 0/O, 1/I/L: codes are read aloud and typed by hand.
const ALPHABET = "23456789ABCDEFGHJKMNPQRSTUVWXYZ";

export default {
  async fetch(req, env) {
    const url = new URL(req.url);
    if (url.pathname === "/") {
      return new Response("Fuzzie relay\n", { headers: { "content-type": "text/plain" } });
    }
    if (url.pathname !== "/register" && url.pathname !== "/ws") {
      return new Response("Not found\n", { status: 404 });
    }
    const hub = env.HUB.get(env.HUB.idFromName("hub"));
    return hub.fetch(req);
  },
};

export class Hub extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.sql.exec(`
      CREATE TABLE IF NOT EXISTS users (
        code TEXT PRIMARY KEY,
        key_hash TEXT NOT NULL,
        name TEXT NOT NULL,
        skin INTEGER NOT NULL DEFAULT 0,
        acc INTEGER NOT NULL DEFAULT 0,
        created INTEGER NOT NULL,
        seen INTEGER NOT NULL
      );
      -- state 0: a asked b to be friends; 1: friends (stored both ways).
      CREATE TABLE IF NOT EXISTS links (
        a TEXT NOT NULL,
        b TEXT NOT NULL,
        state INTEGER NOT NULL,
        PRIMARY KEY (a, b)
      );
      CREATE INDEX IF NOT EXISTS links_b ON links (b);
    `);
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair("ping", "pong"));
    // Last poke per sender; lost on hibernation, which only relaxes the limit.
    this.lastPoke = new Map();
  }

  async fetch(req) {
    const url = new URL(req.url);
    if (url.pathname === "/register" && req.method === "POST") return this.register(req);
    if (url.pathname === "/ws") return this.connect(req);
    return new Response("Not found\n", { status: 404 });
  }

  async register(req) {
    let body = {};
    try {
      body = await req.json();
    } catch {}
    const name = cleanName(body.name) || "Fuzzie";
    const key = hex(crypto.getRandomValues(new Uint8Array(32)));
    const hash = await sha256(key);
    const now = Date.now();
    for (let i = 0; i < 8; i++) {
      const code = newCode();
      if (this.user(code)) continue;
      this.sql.exec("INSERT INTO users (code, key_hash, name, created, seen) VALUES (?, ?, ?, ?, ?)", code, hash, name, now, now);
      return json({ code, key });
    }
    return json({ error: "Try again" }, 503);
  }

  async connect(req) {
    if (req.headers.get("Upgrade") !== "websocket") return new Response("Expected a WebSocket\n", { status: 426 });
    const auth = (req.headers.get("Authorization") || "").replace(/^Bearer\s+/i, "");
    const [code, key] = auth.split(":");
    const u = code && key ? this.user(code) : null;
    if (!u || u.key_hash !== (await sha256(key))) return new Response("Unknown account\n", { status: 401 });

    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    this.ctx.acceptWebSocket(server, [code]);
    server.serializeAttachment({ code, np: null, with: null });
    const wasOnline = this.sockets(code).length > 1;
    this.sql.exec("UPDATE users SET seen = ? WHERE code = ?", Date.now(), code);
    this.sendState(code);
    if (!wasOnline) this.friendsChanged(code);
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws, data) {
    if (typeof data !== "string" || data.length > MAX_FRAME) return;
    let m;
    try {
      m = JSON.parse(data);
    } catch {
      return;
    }
    const me = ws.deserializeAttachment();
    if (!me || !m || typeof m.t !== "string") return;
    const code = me.code;
    switch (m.t) {
      case "hello": {
        const name = cleanName(m.name);
        const skin = smallInt(m.skin);
        const acc = smallInt(m.acc);
        const u = this.user(code);
        if (!u) return;
        if ((name && name !== u.name) || skin !== u.skin || acc !== u.acc) {
          this.sql.exec("UPDATE users SET name = ?, skin = ?, acc = ? WHERE code = ?", name || u.name, skin, acc, code);
          this.sendState(code);
          this.friendsChanged(code);
        }
        break;
      }
      case "np": {
        me.np = cleanTrack(m.np);
        ws.serializeAttachment(me);
        this.friendsChanged(code);
        break;
      }
      case "add": {
        const to = cleanCode(m.code);
        if (!to || to === code) return this.error(ws, "That's your own code");
        if (!this.user(to)) return this.error(ws, "No Fuzzie has that code");
        if (this.link(code, to) === 1) return this.error(ws, "You're already friends");
        if (this.link(to, code) === 0) return this.befriend(code, to); // they asked first
        if (this.count("SELECT COUNT(*) AS n FROM links WHERE a = ? AND state = 0", code) >= MAX_PENDING) return this.error(ws, "Too many open requests");
        if (this.friendCount(code) >= MAX_FRIENDS) return this.error(ws, "Friend list is full");
        this.sql.exec("INSERT OR IGNORE INTO links (a, b, state) VALUES (?, ?, 0)", code, to);
        this.sendState(code);
        this.sendState(to);
        break;
      }
      case "accept": {
        const from = cleanCode(m.code);
        if (from && this.link(from, code) === 0) this.befriend(from, code);
        break;
      }
      case "decline":
      case "remove": {
        const other = cleanCode(m.code);
        if (!other) return;
        this.sql.exec("DELETE FROM links WHERE (a = ? AND b = ?) OR (a = ? AND b = ?)", code, other, other, code);
        this.sendState(code);
        this.sendState(other);
        break;
      }
      case "poke": {
        const to = cleanCode(m.to);
        if (!to || this.link(code, to) !== 1) return;
        const now = Date.now();
        if (now - (this.lastPoke.get(code) || 0) < POKE_GAP_MS) return;
        this.lastPoke.set(code, now);
        const u = this.user(code);
        this.sendTo(to, { t: "poke", from: code, name: u.name, skin: u.skin, acc: u.acc });
        break;
      }
      case "join":
      case "leave": {
        const to = cleanCode(m.to);
        if (!to || this.link(code, to) !== 1) return;
        me.with = m.t === "join" ? to : null;
        ws.serializeAttachment(me);
        this.sendTo(to, { t: m.t, from: code, name: this.user(code).name });
        break;
      }
    }
  }

  async webSocketClose(ws) {
    this.gone(ws);
  }

  async webSocketError(ws) {
    this.gone(ws);
  }

  gone(ws) {
    const me = ws.deserializeAttachment();
    if (!me) return;
    try {
      ws.close(1000, "bye");
    } catch {}
    if (me.with) this.sendTo(me.with, { t: "leave", from: me.code, name: this.user(me.code)?.name || "" });
    if (this.sockets(me.code).filter((s) => s !== ws).length === 0) {
      this.sql.exec("UPDATE users SET seen = ? WHERE code = ?", Date.now(), me.code);
    }
    this.friendsChanged(me.code, ws);
  }

  befriend(a, b) {
    if (this.friendCount(a) >= MAX_FRIENDS || this.friendCount(b) >= MAX_FRIENDS) {
      for (const s of this.sockets(b)) this.error(s, "Friend list is full");
      return;
    }
    this.sql.exec("DELETE FROM links WHERE (a = ? AND b = ?) OR (a = ? AND b = ?)", a, b, b, a);
    this.sql.exec("INSERT INTO links (a, b, state) VALUES (?, ?, 1), (?, ?, 1)", a, b, b, a);
    this.sendState(a);
    this.sendState(b);
  }

  // ---- reads ----

  user(code) {
    return this.sql.exec("SELECT * FROM users WHERE code = ?", code).toArray()[0] || null;
  }

  link(a, b) {
    const r = this.sql.exec("SELECT state FROM links WHERE a = ? AND b = ?", a, b).toArray()[0];
    return r ? r.state : null;
  }

  count(q, ...args) {
    return this.sql.exec(q, ...args).one().n;
  }

  friendCount(code) {
    return this.count("SELECT COUNT(*) AS n FROM links WHERE a = ? AND state = 1", code);
  }

  sockets(code) {
    return this.ctx.getWebSockets(code);
  }

  /// What `code` is playing, from any of its open connections.
  np(code, skip) {
    for (const s of this.sockets(code)) {
      if (s === skip) continue;
      const a = s.deserializeAttachment();
      if (a && a.np) return a.np;
    }
    return null;
  }

  online(code, skip) {
    return this.sockets(code).some((s) => s !== skip);
  }

  // ---- sends ----

  state(code, skip) {
    const me = this.user(code);
    if (!me) return null;
    const friends = this.sql
      .exec("SELECT u.code, u.name, u.skin, u.acc FROM links l JOIN users u ON u.code = l.b WHERE l.a = ? AND l.state = 1 ORDER BY u.name", code)
      .toArray()
      .map((f) => {
        const online = this.online(f.code, skip);
        return { ...f, online, np: online ? this.np(f.code, skip) : null };
      });
    const incoming = this.sql.exec("SELECT u.code, u.name FROM links l JOIN users u ON u.code = l.a WHERE l.b = ? AND l.state = 0", code).toArray();
    const outgoing = this.sql.exec("SELECT u.code, u.name FROM links l JOIN users u ON u.code = l.b WHERE l.a = ? AND l.state = 0", code).toArray();
    return { t: "state", now: Date.now(), me: { code: me.code, name: me.name }, friends, incoming, outgoing };
  }

  sendState(code, skip) {
    if (!this.online(code, skip)) return;
    const s = this.state(code, skip);
    if (s) this.sendTo(code, s, skip);
  }

  /// Tells every online friend of `code` that its presence changed.
  friendsChanged(code, skip) {
    const rows = this.sql.exec("SELECT b FROM links WHERE a = ? AND state = 1", code).toArray();
    for (const r of rows) this.sendState(r.b, skip);
  }

  sendTo(code, msg, skip) {
    const text = JSON.stringify(msg);
    for (const s of this.sockets(code)) {
      if (s === skip) continue;
      try {
        s.send(text);
      } catch {}
    }
  }

  error(ws, msg) {
    try {
      ws.send(JSON.stringify({ t: "error", msg }));
    } catch {}
  }
}

// ---- helpers ----

function json(v, status = 200) {
  return new Response(JSON.stringify(v), { status, headers: { "content-type": "application/json" } });
}

function newCode() {
  const b = crypto.getRandomValues(new Uint8Array(6));
  let s = "FUZ-";
  for (const x of b) s += ALPHABET[x % ALPHABET.length];
  return s;
}

function cleanCode(v) {
  if (typeof v !== "string") return null;
  let s = v.toUpperCase().replace(/[^0-9A-Z]/g, "");
  if (s.length === 9 && s.startsWith("FUZ")) s = s.slice(3);
  return /^[23456789ABCDEFGHJKMNPQRSTUVWXYZ]{6}$/.test(s) ? "FUZ-" + s : null;
}

function cleanText(v, max) {
  if (typeof v !== "string") return "";
  return v.replace(/[\u0000-\u001f\u007f]/g, "").trim().slice(0, max);
}

function cleanName(v) {
  return cleanText(v, 24);
}

function smallInt(v) {
  return Number.isInteger(v) && v >= 0 && v < 1000 ? v : 0;
}

function cleanTrack(np) {
  if (!np || typeof np !== "object") return null;
  const title = cleanText(np.title, 120);
  if (!title) return null;
  const num = (x) => (Number.isFinite(x) && x >= 0 ? Math.min(Math.floor(x), 86400000) : 0);
  return { title, artist: cleanText(np.artist, 120), playing: np.playing === true, pos: num(np.pos), dur: num(np.dur), at: Date.now() };
}

function hex(bytes) {
  return [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
}

async function sha256(s) {
  return hex(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(s))));
}
