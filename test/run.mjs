#!/usr/bin/env node
// The wasmcart-bevy gate: plays every example cart through romdev and checks
// what a player would see and hear. Build first (scripts/build.sh), then
//   node test/run.mjs            # all carts
//   node test/run.mjs lit-scene  # one
// Needs a romdev server (ROMDEV_URL, default http://127.0.0.1:7331/mcp) with
// a WebGPU-capable wasmcart host.
//
// Every cart runs twice: on a core WebGPU device and on a compatibility-mode
// device (the floor a wasmcart host may hand a cart; romdev's
// wgpuFeatureLevel:'compatibility'). Each run asserts which level it got
// (the gpu.core debug field), and the compatibility-mode pictures are
// compared with the core ones: same colours, within a tolerance.
//
// Every behaviour check is paired with a control that must FAIL the same
// check (no input, or nothing that should make a sound, or a core frame shown
// without sRGB encoding for the colour comparison): a check that passes
// either way proves nothing.
import { mkdirSync, rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Romdev, pad, ringLevel } from './romdev.mjs';
import { readPng, distinctColors, diffFraction, countColor } from './png.mjs';

/** The image as a renderer writing linear colour to a non-sRGB canvas shows
 * it (what a compatibility-mode canvas without sRGB showed). */
function unencoded(img) {
  const lin = (v) => { const c = v / 255; return Math.round(255 * (c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4))); };
  const data = Buffer.from(img.data);
  for (let i = 0; i < data.length; i += 4) for (let k = 0; k < 3; k++) data[i + k] = lin(data[i + k]);
  return { ...img, data };
}
/** Mean colour of an image, to compare overall tone. */
function meanColor(img) {
  const m = [0, 0, 0];
  for (let i = 0; i < img.data.length; i += 4) for (let k = 0; k < 3; k++) m[k] += img.data[i + k];
  return m.map((v) => v / (img.w * img.h));
}

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const OUT = join(ROOT, 'test', 'out');
rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });

let failures = 0;
const results = [];
function check(name, ok, detail) {
  results.push({ name, ok, detail });
  if (!ok) failures++;
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? `  (${detail})` : ''}`);
}
/** A control: `pass` must be false, or the check it controls proves nothing. */
const control = (name, pass, detail) => check(`control: ${name} must fail`, !pass, detail);

const near = (a, b, tol) => Math.abs(a - b) <= tol;
const cart = (n) => join(ROOT, 'dist', `${n}.wasc`);
// The level the current run asks for, and its tag in check names and shots.
let LEVEL = 'core';
const tag = () => (LEVEL === 'core' ? '' : ' [compat]');
const shotPath = (name) => join(OUT, LEVEL === 'core' ? `${name}.png` : `${name}.compat.png`);
// Shots kept per level for the core-vs-compat comparison: name -> {core, compatibility}.
const shots = {};
const keep = (key, path) => { (shots[key] ??= {})[LEVEL] = path; };

async function load(rd, name, level = LEVEL) {
  const t0 = performance.now();
  const r = await rd.load(cart(name), level);
  const ms = performance.now() - t0;
  check(`${name}${tag()}: loads`, r.loaded === true, `${ms.toFixed(0)} ms, gpu ${r.gpu ?? 'none'}`);
  return { r, ms, info: (await rd.info()).info };
}

/** The device level the cart rendered on must be the one asked for. */
function checkLevel(name, fields, gpu = true) {
  const want = gpu && LEVEL === 'core' ? 1 : 0;
  const what = want ? 'renders on a core device' : gpu ? 'renders on a compatibility-mode device' : 'reports no GPU device';
  check(`${name}${tag()}: ${what} (gpu.core)`, fields['gpu.core'] === want, `gpu.core ${fields['gpu.core']}`);
}

async function timing(rd, name, frames = 240) {
  try {
    const t = await rd.call('frame', { op: 'timing', frames, warmup: 30 });
    const wall = t.wallMs ?? t.wall ?? t.stats?.wall;
    console.log(`     ${name} frame time: ${JSON.stringify(wall ?? t).slice(0, 200)}`);
    return wall;
  } catch (e) {
    console.log(`     ${name} timing unavailable: ${e.message}`);
    return null;
  }
}

const tests = {
  async 'hello-ecs'(rd) {
    const { info } = await load(rd, 'hello-ecs');
    let r = await rd.play([{ input: pad(), frames: 30 }], ['player_x', 'player_y', 'balls', 'bevy.ready', 'gpu.core']);
    checkLevel('hello-ecs', r.fields, false);
    const x0 = r.fields.player_x;
    check(`hello-ecs${tag()}: ` + 'app ready, player centred', r.fields['bevy.ready'] === 1 && near(x0, 160, 1), `x ${x0}`);
    const quiet = await ringLevel(rd, info, 1600);
    control(`hello-ecs${tag()}: ` + 'sound with no input', quiet.peak > 0.01, `peak ${quiet.peak.toFixed(3)}`);

    r = await rd.play([{ input: pad(), frames: 60 }], ['player_x']);
    control(`hello-ecs${tag()}: ` + 'player moves with no input', !near(r.fields.player_x, x0, 1), `x ${r.fields.player_x}`);

    r = await rd.play([{ input: pad({ right: true }), frames: 60 }], ['player_x']);
    // 120 px/s for one second of host frames
    check(`hello-ecs${tag()}: ` + 'd-pad right moves the player 120 px', near(r.fields.player_x - x0, 120, 3), `dx ${(r.fields.player_x - x0).toFixed(1)}`);

    // a debugging host writes the cart's "speed" field: d-pad left now moves 240 px
    await rd.call('wasm', { op: 'write', name: 'speed', value: 240 });
    r = await rd.play([{ input: pad({ left: true }), frames: 60 }], ['player_x', 'speed']);
    check(`hello-ecs${tag()}: ` + 'a host write to a debug field reaches the cart', near(r.fields.player_x - (x0 + 120), -240, 3) && r.fields.speed === 240, `dx ${(r.fields.player_x - (x0 + 120)).toFixed(1)}, speed ${r.fields.speed}`);
    await rd.call('wasm', { op: 'write', name: 'speed', value: 120 });
    r = await rd.play([{ input: pad({ right: true }), frames: 60 }], ['player_x']);
    control(`hello-ecs${tag()}: ` + 'the 240 px check on a run after 120 was written back', near(r.fields.player_x - (x0 - 120), 240, 3), `dx ${(r.fields.player_x - (x0 - 120)).toFixed(1)}`);

    // keyboard: arrow left for one second moves 120 px too
    r = await rd.play([{ input: pad(), frames: 1 }], ['player_x']);
    const kx = r.fields.player_x;
    r = await rd.play([{ input: pad(), keys: ['left'], frames: 60 }, { input: pad(), frames: 1 }], ['player_x']);
    check(`hello-ecs${tag()}: ` + 'arrow key moves the player 120 px', near(r.fields.player_x - kx, -120, 3), `dx ${(r.fields.player_x - kx).toFixed(1)}`);

    // pointer: hovering does nothing (control), a click puts the player there
    await rd.call('input', { op: 'pointer', x: 60, y: 200 });
    r = await rd.play([{ input: pad(), frames: 2 }], ['player_x', 'player_y']);
    control(`hello-ecs${tag()}: ` + 'hover without a click moves the player', near(r.fields.player_x, 56, 1) && near(r.fields.player_y, 196, 1), `at ${r.fields.player_x},${r.fields.player_y}`);
    await rd.call('input', { op: 'pointer', x: 60, y: 200, left: true });
    r = await rd.play([{ input: pad(), frames: 2 }], ['player_x', 'player_y']);
    await rd.call('input', { op: 'pointer', x: 60, y: 200, left: false });
    check(`hello-ecs${tag()}: ` + 'a click puts the player under the pointer', near(r.fields.player_x, 56, 1) && near(r.fields.player_y, 196, 1), `at ${r.fields.player_x},${r.fields.player_y}`);
    await rd.play([{ input: pad(), frames: 2 }]);

    const shot = shotPath('hello-ecs');
    await rd.shot(shot, 1);
    const white = countColor(readPng(shot), [255, 255, 255], undefined, 2);
    check(`hello-ecs${tag()}: ` + 'the white player square is drawn (8x8)', white === 64, `${white} white px`);

    r = await rd.play([{ input: pad({ south: true }), frames: 2 }], ['balls']);
    const chirp = await ringLevel(rd, info, 1600);
    check(`hello-ecs${tag()}: ` + 'A spawns a square and chirps', r.fields.balls === 1 && chirp.peak > 0.05, `balls ${r.fields.balls}, peak ${chirp.peak.toFixed(3)}`);
  },

  async 'star-catcher'(rd) {
    const { info } = await load(rd, 'star-catcher');
    let r = await rd.play([{ input: pad(), frames: 30 }], ['ship_x', 'ship_y', 'bevy.ready', 'stars', 'gpu.core']);
    check(`star-catcher${tag()}: ` + 'renderer up', r.fields['bevy.ready'] === 1, `stars ${r.fields.stars}`);
    checkLevel('star-catcher', r.fields);
    const x0 = r.fields.ship_x;
    const a = shotPath('star-catcher-a'), b = shotPath('star-catcher-b');
    await rd.shot(a, 1);
    keep('star-catcher', a);
    const ia = readPng(a);
    check(`star-catcher${tag()}: ` + 'frame has content (sprites, text, backdrop)', distinctColors(ia) > 200, `${distinctColors(ia)} colours`);
    // the ship sprite (pale blue) sits bottom-centre
    const shipPx = countColor(ia, [150, 190, 240], { x: 290, y: 280, w: 60, h: 80 }, 60);
    check(`star-catcher${tag()}: ` + 'ship sprite (from the cart assets) is drawn', shipPx > 300, `${shipPx} ship px`);
    const quiet = await ringLevel(rd, info, 1600);
    control(`star-catcher${tag()}: ` + 'sound before anything happens', quiet.peak > 0.01, `peak ${quiet.peak.toFixed(3)}`);

    r = await rd.play([{ input: pad(), frames: 40 }], ['ship_x']);
    control(`star-catcher${tag()}: ` + 'ship moves with no input', !near(r.fields.ship_x, x0, 0.5), `x ${r.fields.ship_x}`);
    await rd.shot(b, 1);
    check(`star-catcher${tag()}: ` + 'the picture moves on its own (stars fall)', diffFraction(ia, readPng(b)) > 0.002, `${(diffFraction(ia, readPng(b)) * 100).toFixed(2)}% px changed`);

    r = await rd.play([{ input: pad({ right: true }), frames: 40 }], ['ship_x']);
    check(`star-catcher${tag()}: ` + 'right moves the ship 173 px', near(r.fields.ship_x - x0, 173.3, 3), `dx ${(r.fields.ship_x - x0).toFixed(1)}`);
    const c = shotPath('star-catcher-c');
    await rd.shot(c, 1);
    const ic = readPng(c);
    const moved = countColor(ic, [150, 190, 240], { x: 460, y: 280, w: 60, h: 80 }, 60);
    check(`star-catcher${tag()}: ` + 'the ship is drawn where it moved', moved > 300, `${moved} ship px at x+173`);

    const sweep = [{ input: pad({ left: true }), frames: 70 }];
    for (let k = 0; k < 16; k++) sweep.push({ input: pad({ left: k % 2 === 1, right: k % 2 === 0 }), frames: 140 });
    r = await rd.play(sweep, ['score', 'lives'], { field: 'score', atLeast: 1 });
    const chime = await ringLevel(rd, info, 1600);
    check(`star-catcher${tag()}: ` + 'catching a star scores and plays the WAV asset', r.fields.score >= 1 && chime.peak > 0.1, `score ${r.fields.score}, peak ${chime.peak.toFixed(3)}`);
  },

  async 'lit-scene'(rd) {
    const { info } = await load(rd, 'lit-scene');
    let r = await rd.play([{ input: pad(), frames: 60 }], ['yaw', 'dist', 'balls', 'bevy.ready', 'gpu.core']);
    check(`lit-scene${tag()}: ` + 'renderer up', r.fields['bevy.ready'] === 1);
    checkLevel('lit-scene', r.fields);
    const yaw0 = r.fields.yaw;
    const a = shotPath('lit-scene-a'), b = shotPath('lit-scene-b'), c = shotPath('lit-scene-c');
    await rd.shot(a, 1);
    keep('lit-scene', a);
    const ia = readPng(a);
    check(`lit-scene${tag()}: ` + 'lit, shaded frame (many colours)', distinctColors(ia) > 5000, `${distinctColors(ia)} colours`);
    const quiet = await ringLevel(rd, info, 1600);
    control(`lit-scene${tag()}: ` + 'sound with no input', quiet.peak > 0.01, `peak ${quiet.peak.toFixed(3)}`);

    r = await rd.play([{ input: pad(), frames: 30 }], ['yaw']);
    control(`lit-scene${tag()}: ` + 'camera orbits with no input', !near(r.fields.yaw, yaw0, 1e-4), `yaw ${r.fields.yaw}`);
    await rd.shot(b, 1);
    const idle = diffFraction(ia, readPng(b));

    r = await rd.play([{ input: pad({ right: true }), frames: 30 }], ['yaw']);
    check(`lit-scene${tag()}: ` + 'right orbits the camera 0.8 rad', near(r.fields.yaw - yaw0, 0.8, 0.02), `dyaw ${(r.fields.yaw - yaw0).toFixed(3)}`);
    await rd.shot(c, 1);
    const orbit = diffFraction(readPng(b), readPng(c));
    check(`lit-scene${tag()}: ` + 'orbiting changes the picture far more than idle animation', orbit > 0.25 && orbit > idle * 2, `orbit ${(orbit * 100).toFixed(1)}% vs idle ${(idle * 100).toFixed(1)}%`);

    r = await rd.play([{ input: pad({ south: true }), frames: 2 }], ['balls']);
    const drop = await ringLevel(rd, info, 1600);
    check(`lit-scene${tag()}: ` + 'A drops a ball with a sound', r.fields.balls === 1 && drop.peak > 0.05, `balls ${r.fields.balls}, peak ${drop.peak.toFixed(3)}`);
    await timing(rd, 'lit-scene');
  },
};

/** Compatibility-mode pictures against core ones: the same scene in the same
 * colours. `minSame` is the share of pixels that must match within `tol`
 * (star-catcher's stars fall at random, so less of its frame can match). */
function compareLevels(name, minSame, tol) {
  const p = shots[name];
  if (!p?.core || !p?.compatibility) return;
  const core = readPng(p.core), compat = readPng(p.compatibility);
  const same = 1 - diffFraction(core, compat, tol);
  const mc = meanColor(core), mx = meanColor(compat);
  const toneOff = Math.max(...mc.map((v, i) => Math.abs(v - mx[i])));
  check(`${name}: compat picture matches core (${(minSame * 100).toFixed(0)}% of pixels within ${tol}, mean colour within 3)`,
    same >= minSame && toneOff <= 3, `${(same * 100).toFixed(1)}% same, mean colour off by ${toneOff.toFixed(2)}`);
  const dark = unencoded(core);
  const ctlSame = 1 - diffFraction(core, dark, tol);
  const ctlTone = Math.max(...mc.map((v, i) => Math.abs(v - meanColor(dark)[i])));
  control(`${name}: a core frame without sRGB encoding matches core`, ctlSame >= minSame && ctlTone <= 3,
    `${(ctlSame * 100).toFixed(1)}% same, mean colour off by ${ctlTone.toFixed(2)}`);
}

/** A prefer_core cart on a compatibility-mode host renders on a core device. */
async function preferCore(rd) {
  LEVEL = 'compatibility';
  const name = 'lit-scene-core';
  const r0 = await rd.load(cart(name), 'compatibility');
  check(`${name} [compat host]: loads`, r0.loaded === true);
  const r = await rd.play([{ input: pad(), frames: 60 }], ['bevy.ready', 'gpu.core']);
  check(`${name} [compat host]: asks for and gets a core device (gpu.core)`, r.fields['bevy.ready'] === 1 && r.fields['gpu.core'] === 1, `ready ${r.fields['bevy.ready']}, gpu.core ${r.fields['gpu.core']}`);
  const a = join(OUT, `${name}.png`);
  await rd.shot(a, 1);
  const img = readPng(a);
  check(`${name} [compat host]: lit, shaded frame (many colours)`, distinctColors(img) > 5000, `${distinctColors(img)} colours`);
}

const only = process.argv[2];
const rd = await new Romdev().connect();
for (const level of ['core', 'compatibility']) {
  LEVEL = level;
  console.log(`── ${level} device ──`);
  for (const [name, fn] of Object.entries(tests)) {
    if (only && only !== name) continue;
    try {
      await fn(rd);
    } catch (e) {
      check(`${name}${tag()}: ran without error`, false, e.message);
    }
  }
}
console.log('── compatibility mode against core ──');
compareLevels('star-catcher', 0.9, 12);
compareLevels('lit-scene', 0.97, 12);
if (!only || only === 'lit-scene') {
  try {
    await preferCore(rd);
  } catch (e) {
    check('lit-scene-core: ran without error', false, e.message);
  }
}
console.log(`\n${results.length - failures}/${results.length} checks passed`);
process.exit(failures ? 1 : 0);
