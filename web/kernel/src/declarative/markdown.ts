/**
 * The UI kernel's markdown renderer (07-ui-plugins, `ctx.ui.markdown`). Raw HTML is
 * shown as text, images as their alt text (no request leaves the app), and links are
 * marked so the kernel copies them instead of navigating (06-client, UI rules).
 */
import { Marked, type Tokens } from 'marked';

const ESCAPES: Record<string, string> = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };
export const escapeHtml = (s: string): string => s.replace(/[&<>"']/g, (c) => ESCAPES[c]!);

/** Link targets the kernel offers to copy; anything else is shown as plain text. */
export function safeHref(href: string): string | undefined {
  try {
    const u = new URL(href);
    return ['http:', 'https:', 'mailto:'].includes(u.protocol) ? u.href : undefined;
  } catch {
    return undefined;
  }
}

const marked = new Marked({
  gfm: true,
  breaks: false,
  async: false,
  renderer: {
    html(token: Tokens.HTML | Tokens.Tag) {
      return escapeHtml(token.text);
    },
    image(token: Tokens.Image) {
      return escapeHtml(token.text);
    },
    link(this: { parser: { parseInline(t: Tokens.Generic[]): string } }, token: Tokens.Link) {
      const text = this.parser.parseInline(token.tokens);
      const href = safeHref(token.href);
      if (!href) return text;
      return `<a href="${escapeHtml(href)}" data-kk-link title="Copy link: ${escapeHtml(href)}">${text}</a>`;
    },
  },
});

/** Markdown as safe HTML. */
export function markdownToHtml(text: string): string {
  return marked.parse(text) as string;
}

/** Renders markdown into `el`. */
export function renderMarkdown(el: HTMLElement, text: string): void {
  el.innerHTML = markdownToHtml(text);
}
