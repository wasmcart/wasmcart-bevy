// A minimal MCP client for a romdev server (default http://127.0.0.1:7331/mcp),
// so the test gate drives carts through romdev: load, play with the pad, read
// named debug fields, screenshot, read cart memory.
export class Romdev {
  constructor(url = process.env.ROMDEV_URL ?? 'http://127.0.0.1:7331/mcp') {
    this.url = url;
    this.id = 0;
    this.sid = null;
  }
  async #post(body) {
    const headers = { 'content-type': 'application/json', accept: 'application/json, text/event-stream' };
    if (this.sid) headers['mcp-session-id'] = this.sid;
    const res = await fetch(this.url, { method: 'POST', headers, body: JSON.stringify(body) });
    if (!this.sid) this.sid = res.headers.get('mcp-session-id');
    const text = await res.text();
    if (!text) return null;
    // streamable HTTP may answer as SSE: take the last data: line
    const json = text.startsWith('{') ? text : text.split('\n').filter((l) => l.startsWith('data:')).map((l) => l.slice(5)).pop();
    return JSON.parse(json);
  }
  async connect() {
    await this.#post({ jsonrpc: '2.0', id: ++this.id, method: 'initialize',
      params: { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'wasmcart-bevy-test', version: '1' } } });
    await this.#post({ jsonrpc: '2.0', method: 'notifications/initialized' });
    return this;
  }
  /** Call a tool; returns the parsed JSON of its first text block (or the text). */
  async call(name, args) {
    const r = await this.#post({ jsonrpc: '2.0', id: ++this.id, method: 'tools/call', params: { name, arguments: args } });
    if (r.error) throw new Error(`${name}: ${r.error.message}`);
    const text = (r.result.content ?? []).filter((c) => c.type === 'text').map((c) => c.text)[0] ?? '';
    if (r.result.isError) throw new Error(`${name}: ${text}`);
    const body = text.replace(/\nsession: .*$/s, '').replace(/session: [0-9a-f-]+ - .*$/s, '');
    try { return JSON.parse(body); } catch { return body; }
  }
  load(path) { return this.call('loadMedia', { platform: 'wasmcart', path }); }
  play(steps, fields, stopWhen) { return this.call('input', { op: 'sequence', steps, fields, ...(stopWhen && { stopWhen }) }); }
  shot(path, frames = 1) { return this.call('frame', { op: 'stepAndShot', frames, path }); }
  async read(offset, length) {
    const r = await this.call('wasm', { op: 'read', offset, length, inline: true });
    return Buffer.from(r.hex, 'hex');
  }
  info() { return this.call('wasm', { op: 'info' }); }
}

/** Pad state for one port, e.g. pad({right:true, south:true}). */
export const pad = (p = {}) => ({ ports: [p] });

/** Peak and RMS of the last `frames` stereo frames written to the cart's f32 ring. */
export async function ringLevel(rd, info, frames) {
  const cursor = (await rd.read(info.audioWritePtr, 4)).readUInt32LE(0);
  const cap = info.audioCap;
  let peak = 0, sum = 0;
  const n = frames;
  // at most two contiguous runs
  const start = (cursor - frames + cap) % cap;
  const runs = start + frames <= cap ? [[start, frames]] : [[start, cap - start], [0, frames - (cap - start)]];
  for (const [s, len] of runs) {
    const buf = await rd.read(info.audioPtr + s * 8, len * 8);
    for (let i = 0; i < len * 2; i++) {
      const v = buf.readFloatLE(i * 4);
      peak = Math.max(peak, Math.abs(v));
      sum += v * v;
    }
  }
  return { cursor, peak, rms: Math.sqrt(sum / (n * 2)) };
}
