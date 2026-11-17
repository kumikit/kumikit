import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

import { respectGdkBackend, squashfsInfo } from './repack-appimage.mjs';

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
