#!/usr/bin/env node
// Print a cart's import and export lists and fail if it imports from a module
// a wasmcart host does not provide (anything but env, wgpu, gl, wasi*).
import { readFileSync } from 'node:fs';
const file = process.argv[2];
if (!file) { console.error('usage: wasm-imports.mjs cart.wasm'); process.exit(2); }
const mod = new WebAssembly.Module(readFileSync(file));
const imports = WebAssembly.Module.imports(mod);
const exports = WebAssembly.Module.exports(mod);
const byModule = {};
for (const i of imports) (byModule[i.module] ??= []).push(i.name);
for (const [m, names] of Object.entries(byModule)) console.log(`import ${m} (${names.length}): ${names.join(' ')}`);
console.log(`exports: ${exports.map((e) => e.name).join(' ')}`);
const bad = Object.keys(byModule).filter((m) => !/^(env|wgpu|gl|wasi.*)$/.test(m));
if (bad.length) { console.error(`FAIL: imports from modules no wasmcart host provides: ${bad.join(', ')}`); process.exit(1); }
const bindgen = imports.filter((i) => /wbindgen|wbg/.test(i.module + i.name));
if (bindgen.length) { console.error(`FAIL: wasm-bindgen imports: ${bindgen.map((i) => i.name).join(' ')}`); process.exit(1); }
console.log('OK: only wasmcart import modules');
