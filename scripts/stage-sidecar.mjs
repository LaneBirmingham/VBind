// Copy the release `vbind` CLI to where Tauri expects the sidecar binary, so
// the installer puts vbind.exe next to VBind.exe for agents to call.
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const triple = "x86_64-pc-windows-msvc";
const from = join(root, "target", "release", "vbind.exe");
const to = join(root, "crates", "vbind-app", "binaries", `vbind-${triple}.exe`);
mkdirSync(dirname(to), { recursive: true });
copyFileSync(from, to);
console.log(`staged ${from} -> ${to}`);
