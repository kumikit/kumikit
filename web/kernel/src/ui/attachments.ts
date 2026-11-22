/**
 * Client-side attachment helpers for composers: reads File / clipboard items into
 * `@kumikit/protocol` `Attachment` objects with base64 payloads and image dimensions.
 */
import type { Attachment } from '@kumikit/protocol';

export const MAX_ATTACHMENTS = 16;
export const MAX_ATTACHMENT_BYTES = 4 * 1024 * 1024;

export interface PendingAttachment extends Attachment {
  /** Local unique key for list rendering. */
  id: string;
  /** Original decoded byte size, for display. */
  size: number;
  /** Data URL preview for images. */
  previewUrl?: string;
}

let nextAttachmentId = 0;

/** Formats a byte count into a short human-readable string (e.g. `4.2 KB`). */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Reads an image's natural pixel dimensions from a data URL. */
function measureImage(dataUrl: string): Promise<{ width: number; height: number } | undefined> {
  if (typeof Image === 'undefined') return Promise.resolve(undefined);
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => resolve({ width: img.naturalWidth, height: img.naturalHeight });
    img.onerror = () => resolve(undefined);
    img.src = dataUrl;
  });
}

function readAsDataUrl(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === 'string') resolve(reader.result);
      else reject(new Error('Could not read file'));
    };
    reader.onerror = () => reject(reader.error ?? new Error('Could not read file'));
    reader.readAsDataURL(file);
  });
}

function defaultPastedName(mimeType: string, existing: readonly PendingAttachment[]): string {
  const ext = mimeType.split('/')[1]?.split('+')[0]?.replace('jpeg', 'jpg') || 'png';
  const base = mimeType.startsWith('image/') ? 'Pasted image' : 'Pasted file';
  const candidate = `${base}.${ext}`;
  if (!existing.some((a) => a.name === candidate)) return candidate;
  for (let n = 2; n < 100; n++) {
    const next = `${base} ${n}.${ext}`;
    if (!existing.some((a) => a.name === next)) return next;
  }
  return `${base}-${Date.now()}.${ext}`;
}

/** Extracts attached files (including pasted screenshots) from a ClipboardEvent. */
export function filesFromClipboard(event: ClipboardEvent): File[] {
  const dt = event.clipboardData;
  if (!dt) return [];
  const out: File[] = [];
  if (dt.items && dt.items.length > 0) {
    for (const item of Array.from(dt.items)) {
      if (item.kind === 'file') {
        const f = item.getAsFile();
        if (f) out.push(f);
      }
    }
  }
  if (out.length === 0 && dt.files && dt.files.length > 0) {
    out.push(...Array.from(dt.files));
  }
  return out;
}

/** Reads `files` into pending attachments while enforcing the per-message count and size limits. */
export async function readAttachments(
  files: readonly File[],
  existing: readonly PendingAttachment[],
  pasted = false,
): Promise<{ added: PendingAttachment[]; error?: string }> {
  let totalBytes = existing.reduce((sum, a) => sum + a.size, 0);
  const added: PendingAttachment[] = [];
  let error: string | undefined;

  for (const file of files) {
    if (existing.length + added.length >= MAX_ATTACHMENTS) {
      error = `At most ${MAX_ATTACHMENTS} attachments per message.`;
      break;
    }
    if (totalBytes + file.size > MAX_ATTACHMENT_BYTES) {
      error = 'Attachments cannot exceed 4 MB per message.';
      continue;
    }
    const mimeType = file.type || 'application/octet-stream';
    const rawName = file.name?.trim();
    const name =
      !rawName || (pasted && /^image\.\w+$/i.test(rawName))
        ? defaultPastedName(mimeType, [...existing, ...added])
        : rawName;
    const dataUrl = await readAsDataUrl(file);
    const comma = dataUrl.indexOf(',');
    const data = comma >= 0 ? dataUrl.slice(comma + 1) : '';
    const item: PendingAttachment = {
      id: `att-${++nextAttachmentId}`,
      name,
      mimeType,
      data,
      size: file.size,
    };
    if (mimeType.startsWith('image/')) {
      item.previewUrl = dataUrl;
      const dims = await measureImage(dataUrl);
      if (dims && dims.width > 0 && dims.height > 0) {
        item.width = dims.width;
        item.height = dims.height;
      }
    }
    added.push(item);
    totalBytes += file.size;
  }

  return error ? { added, error } : { added };
}

/** Converts files returned by the native OS attachment picker into PendingAttachments. */
export async function fromPickedAttachments(
  picked: readonly { name: string; mimeType: string; data: string; size: number }[],
  existing: readonly PendingAttachment[],
): Promise<{ added: PendingAttachment[]; error?: string }> {
  let totalBytes = existing.reduce((sum, a) => sum + a.size, 0);
  const added: PendingAttachment[] = [];
  let error: string | undefined;

  for (const file of picked) {
    if (existing.length + added.length >= MAX_ATTACHMENTS) {
      error = `At most ${MAX_ATTACHMENTS} attachments per message.`;
      break;
    }
    if (totalBytes + file.size > MAX_ATTACHMENT_BYTES) {
      error = 'Attachments cannot exceed 4 MB per message.';
      continue;
    }
    const item: PendingAttachment = {
      id: `att-${++nextAttachmentId}`,
      name: file.name,
      mimeType: file.mimeType,
      data: file.data,
      size: file.size,
    };
    if (file.mimeType.startsWith('image/')) {
      const dataUrl = `data:${file.mimeType};base64,${file.data}`;
      item.previewUrl = dataUrl;
      const dims = await measureImage(dataUrl);
      if (dims && dims.width > 0 && dims.height > 0) {
        item.width = dims.width;
        item.height = dims.height;
      }
    }
    added.push(item);
    totalBytes += file.size;
  }

  return error ? { added, error } : { added };
}

/** Strips UI-only fields before sending over JSON-RPC. */
export function toProtocolAttachments(items: readonly PendingAttachment[]): Attachment[] {
  return items.map((a) => {
    const out: Attachment = { name: a.name, mimeType: a.mimeType, data: a.data };
    if (a.width !== undefined) out.width = a.width;
    if (a.height !== undefined) out.height = a.height;
    return out;
  });
}

/** Builds a safe data URL for displaying an image attachment in the transcript. */
export function attachmentImageUrl(a: Attachment): string | undefined {
  if (!a.mimeType.startsWith('image/')) return undefined;
  return `data:${a.mimeType};base64,${a.data}`;
}
