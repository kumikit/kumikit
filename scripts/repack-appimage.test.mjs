import { execFileSync } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

import { installCliDispatch, respectGdkBackend, squashfsInfo } from './repack-appimage.mjs';

describe('the GTK hook', () => {
  it('makes X11 a default the user can override, in every form Tauri has shipped', () => {
    const want = 'export GDK_BACKEND="${GDK_BACKEND:-x11}"';
    expect(respectGdkBackend('export GDK_BACKEND=x11')).toBe(want);
    expect(respectGdkBackend('  export GDK_BACKEND=x11 # forced')).toBe(`  ${want}`);
    expect(respectGdkBackend('# export GDK_BACKEND=x11 # Monitor this closely. AppImage used to crash on Wayland!')).toBe(want);
    expect(respectGdkBackend(`a\n${want}\nb`)).toBe(`a\n${want}\nb`);
  });

  it('refuses a hook it does not recognize', () => {
    expect(() => respectGdkBackend('export OTHER=1')).toThrow(/no longer sets GDK_BACKEND/);
  });
});

describe('the squashfs header', () => {
  it('gives the compression and block size to pack with', () => {
    const file = join(mkdtempSync(join(tmpdir(), 'kk-sq-')), 'img');
    const header = Buffer.alloc(96);
    header.writeUInt32LE(0x73717368, 0);
    header.writeUInt32LE(131072, 12);
    header.writeUInt16LE(6, 20);
    writeFileSync(file, Buffer.concat([Buffer.alloc(100), header]));
    expect(squashfsInfo(file, 100)).toEqual({ compression: 'zstd', blockSize: 131072 });
    expect(() => squashfsInfo(file, 0)).toThrow(/no squashfs image/);
  });
});

describe('AppRun CLI dispatch', () => {
  const makeAppDir = () => {
    const root = mkdtempSync(join(tmpdir(), 'kk-apprun-'));
    mkdirSync(join(root, 'usr/bin'), { recursive: true });
    writeFileSync(join(root, 'AppRun'), '#!/bin/sh\necho "DESKTOP:$*"\n');
    chmodSync(join(root, 'AppRun'), 0o755);
    writeFileSync(join(root, 'usr/bin/kumikit'), '#!/bin/sh\nprintf "CLI:"; printf "<%s>" "$@"; printf "\\n"\n');
    chmodSync(join(root, 'usr/bin/kumikit'), 0o755);
    return root;
  };

  it.skipIf(process.platform === 'win32')('launches desktop with no args or --safe and forwards all other args to kumikit', () => {
    const root = makeAppDir();
    installCliDispatch(root);
    // Idempotent on a second repack: AppRun.desktop keeps the original desktop script.
    installCliDispatch(root);
    expect(readFileSync(join(root, 'AppRun.desktop'), 'utf8')).toContain('DESKTOP:');

    const run = (args) => execFileSync(join(root, 'AppRun'), args, { encoding: 'utf8' }).trim();
    expect(run([])).toBe('DESKTOP:');
    expect(run(['--safe'])).toBe('DESKTOP:--safe');
    expect(run(['--help'])).toBe('CLI:<--help>');
    expect(run(['--version'])).toBe('CLI:<--version>');
    expect(run(['plugin', 'list', '--socket', '/tmp/a b.sock'])).toBe('CLI:<plugin><list><--socket></tmp/a b.sock>');
    expect(run(['--safe', 'serve'])).toBe('CLI:<--safe><serve>');
  });

  it('refuses an AppDir missing usr/bin/kumikit or AppRun', () => {
    const empty = mkdtempSync(join(tmpdir(), 'kk-apprun-empty-'));
    expect(() => installCliDispatch(empty)).toThrow(/bundled CLI binary missing/);
    mkdirSync(join(empty, 'usr/bin'), { recursive: true });
    writeFileSync(join(empty, 'usr/bin/kumikit'), '');
    expect(() => installCliDispatch(empty)).toThrow(/AppRun missing/);
  });
});
