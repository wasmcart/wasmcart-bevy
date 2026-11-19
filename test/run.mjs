#!/usr/bin/env node
// The wasmcart-bevy gate: plays every example cart through romdev and checks
// what a player would see and hear. Build first (scripts/build.sh), then
//   node test/run.mjs            # all carts
//   node test/run.mjs lit-scene  # one
// Needs a romdev server (ROMDEV_URL, default http://127.0.0.1:7331/mcp) with
// a WebGPU-capable wasmcart host.
//
// Every behaviour check is paired with a control that must FAIL the same
// check (no input, or nothing that should make a sound): a check that passes
// either way proves nothing.
import { mkdirSync, rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Romdev, pad, ringLevel } from './romdev.mjs';
import { readPng, distinctColors, diffFraction, countColor } from './png.mjs';

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

async function load(rd, name) {
  const t0 = performance.now();
  const r = await rd.load(cart(name));
  const ms = performance.now() - t0;
  check(`${name}: loads`, r.loaded === true, `${ms.toFixed(0)} ms, gpu ${r.gpu ?? 'none'}`);
  return { r, ms, info: (await rd.info()).info };
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
    let r = await rd.play([{ input: pad(), frames: 30 }], ['player_x', 'player_y', 'balls', 'bevy.ready']);
    const x0 = r.fields.player_x;
    check('hello-ecs: app ready, player centred', r.fields['bevy.ready'] === 1 && near(x0, 160, 1), `x ${x0}`);
    const quiet = await ringLevel(rd, info, 1600);
    control('hello-ecs: sound with no input', quiet.peak > 0.01, `peak ${quiet.peak.toFixed(3)}`);

    r = await rd.play([{ input: pad(), frames: 60 }], ['player_x']);
    control('hello-ecs: player moves with no input', !near(r.fields.player_x, x0, 1), `x ${r.fields.player_x}`);

    r = await rd.play([{ input: pad({ right: true }), frames: 60 }], ['player_x']);
    // 120 px/s for one second of host frames
    check('hello-ecs: d-pad right moves the player 120 px', near(r.fields.player_x - x0, 120, 3), `dx ${(r.fields.player_x - x0).toFixed(1)}`);

    // a debugging host writes the cart's "speed" field: d-pad left now moves 240 px
    await rd.call('wasm', { op: 'write', name: 'speed', value: 240 });
    r = await rd.play([{ input: pad({ left: true }), frames: 60 }], ['player_x', 'speed']);
    check('hello-ecs: a host write to a debug field reaches the cart', near(r.fields.player_x - (x0 + 120), -240, 3) && r.fields.speed === 240, `dx ${(r.fields.player_x - (x0 + 120)).toFixed(1)}, speed ${r.fields.speed}`);
    await rd.call('wasm', { op: 'write', name: 'speed', value: 120 });
    r = await rd.play([{ input: pad({ right: true }), frames: 60 }], ['player_x']);
    control('hello-ecs: the 240 px check on a run after 120 was written back', near(r.fields.player_x - (x0 - 120), 240, 3), `dx ${(r.fields.player_x - (x0 - 120)).toFixed(1)}`);

    // keyboard: arrow left for one second moves 120 px too
    r = await rd.play([{ input: pad(), frames: 1 }], ['player_x']);
    const kx = r.fields.player_x;
    r = await rd.play([{ input: pad(), keys: ['left'], frames: 60 }, { input: pad(), frames: 1 }], ['player_x']);
    check('hello-ecs: arrow key moves the player 120 px', near(r.fields.player_x - kx, -120, 3), `dx ${(r.fields.player_x - kx).toFixed(1)}`);

    // pointer: hovering does nothing (control), a click puts the player there
    await rd.call('input', { op: 'pointer', x: 60, y: 200 });
    r = await rd.play([{ input: pad(), frames: 2 }], ['player_x', 'player_y']);
    control('hello-ecs: hover without a click moves the player', near(r.fields.player_x, 56, 1) && near(r.fields.player_y, 196, 1), `at ${r.fields.player_x},${r.fields.player_y}`);
    await rd.call('input', { op: 'pointer', x: 60, y: 200, left: true });
    r = await rd.play([{ input: pad(), frames: 2 }], ['player_x', 'player_y']);
    await rd.call('input', { op: 'pointer', x: 60, y: 200, left: false });
    check('hello-ecs: a click puts the player under the pointer', near(r.fields.player_x, 56, 1) && near(r.fields.player_y, 196, 1), `at ${r.fields.player_x},${r.fields.player_y}`);
    await rd.play([{ input: pad(), frames: 2 }]);

    const shot = join(OUT, 'hello-ecs.png');
    await rd.shot(shot, 1);
    const white = countColor(readPng(shot), [255, 255, 255], undefined, 2);
    check('hello-ecs: the white player square is drawn (8x8)', white === 64, `${white} white px`);

    r = await rd.play([{ input: pad({ south: true }), frames: 2 }], ['balls']);
    const chirp = await ringLevel(rd, info, 1600);
    check('hello-ecs: A spawns a square and chirps', r.fields.balls === 1 && chirp.peak > 0.05, `balls ${r.fields.balls}, peak ${chirp.peak.toFixed(3)}`);
  },

  async 'star-catcher'(rd) {
    const { info } = await load(rd, 'star-catcher');
    let r = await rd.play([{ input: pad(), frames: 30 }], ['ship_x', 'ship_y', 'bevy.ready', 'stars']);
    check('star-catcher: renderer up', r.fields['bevy.ready'] === 1, `stars ${r.fields.stars}`);
    const x0 = r.fields.ship_x;
    const a = join(OUT, 'star-catcher-a.png'), b = join(OUT, 'star-catcher-b.png');
    await rd.shot(a, 1);
    const ia = readPng(a);
    check('star-catcher: frame has content (sprites, text, backdrop)', distinctColors(ia) > 200, `${distinctColors(ia)} colours`);
    // the ship sprite (pale blue) sits bottom-centre
    const shipPx = countColor(ia, [150, 190, 240], { x: 290, y: 280, w: 60, h: 80 }, 60);
    check('star-catcher: ship sprite (from the cart assets) is drawn', shipPx > 300, `${shipPx} ship px`);
    const quiet = await ringLevel(rd, info, 1600);
    control('star-catcher: sound before anything happens', quiet.peak > 0.01, `peak ${quiet.peak.toFixed(3)}`);

    r = await rd.play([{ input: pad(), frames: 40 }], ['ship_x']);
    control('star-catcher: ship moves with no input', !near(r.fields.ship_x, x0, 0.5), `x ${r.fields.ship_x}`);
    await rd.shot(b, 1);
    check('star-catcher: the picture moves on its own (stars fall)', diffFraction(ia, readPng(b)) > 0.002, `${(diffFraction(ia, readPng(b)) * 100).toFixed(2)}% px changed`);

    r = await rd.play([{ input: pad({ right: true }), frames: 40 }], ['ship_x']);
    check('star-catcher: right moves the ship 173 px', near(r.fields.ship_x - x0, 173.3, 3), `dx ${(r.fields.ship_x - x0).toFixed(1)}`);
    const c = join(OUT, 'star-catcher-c.png');
    await rd.shot(c, 1);
    const ic = readPng(c);
    const moved = countColor(ic, [150, 190, 240], { x: 460, y: 280, w: 60, h: 80 }, 60);
    check('star-catcher: the ship is drawn where it moved', moved > 300, `${moved} ship px at x+173`);

    const sweep = [{ input: pad({ left: true }), frames: 70 }];
    for (let k = 0; k < 16; k++) sweep.push({ input: pad({ left: k % 2 === 1, right: k % 2 === 0 }), frames: 140 });
    r = await rd.play(sweep, ['score', 'lives'], { field: 'score', atLeast: 1 });
    const chime = await ringLevel(rd, info, 1600);
    check('star-catcher: catching a star scores and plays the WAV asset', r.fields.score >= 1 && chime.peak > 0.1, `score ${r.fields.score}, peak ${chime.peak.toFixed(3)}`);
  },

  async 'lit-scene'(rd) {
    const { info } = await load(rd, 'lit-scene');
    let r = await rd.play([{ input: pad(), frames: 60 }], ['yaw', 'dist', 'balls', 'bevy.ready']);
    check('lit-scene: renderer up', r.fields['bevy.ready'] === 1);
    const yaw0 = r.fields.yaw;
    const a = join(OUT, 'lit-scene-a.png'), b = join(OUT, 'lit-scene-b.png'), c = join(OUT, 'lit-scene-c.png');
    await rd.shot(a, 1);
    const ia = readPng(a);
    check('lit-scene: lit, shaded frame (many colours)', distinctColors(ia) > 5000, `${distinctColors(ia)} colours`);
    const quiet = await ringLevel(rd, info, 1600);
    control('lit-scene: sound with no input', quiet.peak > 0.01, `peak ${quiet.peak.toFixed(3)}`);

    r = await rd.play([{ input: pad(), frames: 30 }], ['yaw']);
    control('lit-scene: camera orbits with no input', !near(r.fields.yaw, yaw0, 1e-4), `yaw ${r.fields.yaw}`);
    await rd.shot(b, 1);
    const idle = diffFraction(ia, readPng(b));

    r = await rd.play([{ input: pad({ right: true }), frames: 30 }], ['yaw']);
    check('lit-scene: right orbits the camera 0.8 rad', near(r.fields.yaw - yaw0, 0.8, 0.02), `dyaw ${(r.fields.yaw - yaw0).toFixed(3)}`);
    await rd.shot(c, 1);
    const orbit = diffFraction(readPng(b), readPng(c));
    check('lit-scene: orbiting changes the picture far more than idle animation', orbit > 0.25 && orbit > idle * 2, `orbit ${(orbit * 100).toFixed(1)}% vs idle ${(idle * 100).toFixed(1)}%`);

    r = await rd.play([{ input: pad({ south: true }), frames: 2 }], ['balls']);
    const drop = await ringLevel(rd, info, 1600);
    check('lit-scene: A drops a ball with a sound', r.fields.balls === 1 && drop.peak > 0.05, `balls ${r.fields.balls}, peak ${drop.peak.toFixed(3)}`);
    await timing(rd, 'lit-scene');
  },
};

const only = process.argv[2];
const rd = await new Romdev().connect();
for (const [name, fn] of Object.entries(tests)) {
  if (only && only !== name) continue;
  try {
    await fn(rd);
  } catch (e) {
    check(`${name}: ran without error`, false, e.message);
  }
}
console.log(`\n${results.length - failures}/${results.length} checks passed`);
process.exit(failures ? 1 : 0);
