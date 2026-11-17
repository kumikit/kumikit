/** Writes a `.kkp` (tar compressed with zstd, 04-plugin-host) from in-memory files, for
 * client plugin packages in tests. */
import { writeFileSync } from 'node:fs';
import { zstdCompressSync } from 'node:zlib';

function header(name: string, size: number): Buffer {
  const h = Buffer.alloc(512);
  h.write(name, 0, 100, 'utf8');
  h.write('0000644\0', 100);
  h.write('0000000\0', 108);
  h.write('0000000\0', 116);
  h.write(size.toString(8).padStart(11, '0') + '\0', 124);
  h.write('00000000000\0', 136);
  h.write('        ', 148);
  h.write('0', 156);
  h.write('ustar\0', 257);
  h.write('00', 263);
  let sum = 0;
  for (const b of h) sum += b;
  h.write(sum.toString(8).padStart(6, '0') + '\0 ', 148);
  return h;
}

export function writeKkp(path: string, files: Record<string, string>): string {
  const parts: Buffer[] = [];
  for (const [name, text] of Object.entries(files).sort(([a], [b]) => a.localeCompare(b))) {
    const body = Buffer.from(text, 'utf8');
    parts.push(header(name, body.length), body, Buffer.alloc((512 - (body.length % 512)) % 512));
  }
  parts.push(Buffer.alloc(1024));
  writeFileSync(path, zstdCompressSync(Buffer.concat(parts)));
  return path;
}
