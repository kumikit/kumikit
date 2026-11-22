// Fixes two things in the AppImage Tauri builds:
//
// 1. Tauri copies the build host's Wayland libraries into usr/lib. On hosts with Mesa 25
//    or newer, the host's libEGL then loads against these older copies, WebKit's web
//    process cannot create its EGL display, and the window stays blank. Without them
//    the AppImage uses the host's Wayland and EGL stack, as it already does for libGL
//    and GPU drivers.
// 2. linuxdeploy's GTK hook sets GDK_BACKEND=x11 unconditionally. It becomes a default,
//    so a user whose Xwayland cannot show the window can run with GDK_BACKEND=wayland.
//
// The image is extracted, changed and packed again with the same runtime, compression
// and block size. Needs squashfs-tools (unsquashfs, mksquashfs).
// Usage: node scripts/repack-appimage.mjs <file.AppImage>
import { execFileSync } from 'node:child_process';
import { chmodSync, closeSync, mkdtempSync, openSync, readFileSync, readSync, readdirSync, renameSync, rmSync, writeFileSync, writeSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const WAYLAND = /^libwayland-(?:client|cursor|egl|server)\.so(?:\.|$)/;
const HOOK = 'apprun-hooks/linuxdeploy-plugin-gtk.sh';
// Forced (`export GDK_BACKEND=x11`), or commented out as Tauri 2.12 ships it, possibly
// with a trailing comment.
const FORCED = /^([ \t]*)(?:#[ \t]*)?export GDK_BACKEND=x11(?=[ \t]*(?:#.*)?$).*$/m;
const DEFAULT = 'export GDK_BACKEND="${GDK_BACKEND:-x11}"';
// squashfs superblock compression ids, as mksquashfs names them.
const COMPRESSION = { 1: 'gzip', 2: 'lzma', 3: 'lzo', 4: 'xz', 5: 'lz4', 6: 'zstd' };

/** Where the runtime ends and the squashfs image starts, and how the image is packed. */
export function squashfsInfo(file, offset) {
  const fd = openSync(file, 'r');
  const header = Buffer.alloc(96);
  readSync(fd, header, 0, 96, offset);
  closeSync(fd);
  if (header.readUInt32LE(0) !== 0x73717368) throw new Error('no squashfs image at the runtime offset');
  const compression = COMPRESSION[header.readUInt16LE(20)];
  const blockSize = header.readUInt32LE(12);
  if (!compression) throw new Error(`unknown squashfs compression ${header.readUInt16LE(20)}`);
  return { compression, blockSize };
}

/** The GTK hook with X11 as a default; unchanged if it already is one. */
export function respectGdkBackend(script) {
  if (script.includes(DEFAULT)) return script;
  if (!FORCED.test(script)) throw new Error(`${HOOK} no longer sets GDK_BACKEND=x11; update scripts/repack-appimage.mjs`);
  return script.replace(FORCED, (_, indent) => `${indent}${DEFAULT}`);
}

export function repack(appImage) {
  const file = resolve(appImage);
  const run = (cmd, args, opts = {}) => execFileSync(cmd, args, { stdio: ['ignore', 'pipe', 'inherit'], ...opts }).toString();
  // The runtime can report its offset without FUSE.
  const offset = Number(run(file, ['--appimage-offset'], { env: { ...process.env, APPIMAGE_EXTRACT_AND_RUN: '1' } }).trim());
  if (!Number.isSafeInteger(offset) || offset <= 0) throw new Error('cannot read the AppImage runtime offset');
  const { compression, blockSize } = squashfsInfo(file, offset);
  const work = mkdtempSync(join(tmpdir(), 'kumikit-appimage-'));
  try {
    const root = join(work, 'root');
    run('unsquashfs', ['-quiet', '-no-progress', '-offset', String(offset), '-dest', root, file]);
    const removed = [];
    for (const dir of ['usr/lib', 'usr/lib64', 'usr/lib/x86_64-linux-gnu', 'usr/lib/aarch64-linux-gnu']) {
      let names = [];
      try {
        names = readdirSync(join(root, dir));
      } catch {
        continue;
      }
      for (const name of names.filter((n) => WAYLAND.test(n))) {
        rmSync(join(root, dir, name), { force: true });
        removed.push(`${dir}/${name}`);
      }
    }
    const hook = join(root, HOOK);
    const script = readFileSync(hook, 'utf8');
    writeFileSync(hook, respectGdkBackend(script));
    const image = join(work, 'image.squashfs');
    run('mksquashfs', [root, image, '-noappend', '-root-owned', '-quiet', '-no-progress', '-comp', compression, '-b', String(blockSize)]);
    // The original runtime, then the new image.
    const out = `${file}.repacked`;
    const runtime = Buffer.alloc(offset);
    const fd = openSync(file, 'r');
    readSync(fd, runtime, 0, offset, 0);
    closeSync(fd);
    const outFd = openSync(out, 'w');
    writeSync(outFd, runtime);
    writeSync(outFd, readFileSync(image));
    closeSync(outFd);
    chmodSync(out, 0o755);
    renameSync(out, file);
    console.log(`repacked ${file}: removed ${removed.length ? removed.join(', ') : 'no Wayland libraries'}; GDK_BACKEND is now a default`);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(new URL(import.meta.url).pathname)) {
  const file = process.argv[2];
  if (!file) {
    console.error('usage: node scripts/repack-appimage.mjs <file.AppImage>');
    process.exit(2);
  }
  repack(file);
}
