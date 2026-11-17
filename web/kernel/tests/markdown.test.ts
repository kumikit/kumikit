// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';

import { markdownToHtml, renderMarkdown, safeHref } from '../src/declarative/markdown';

describe('markdown', () => {
  it('renders the usual things', () => {
    const html = markdownToHtml('# Title\n\nSome **bold** and `code`.\n\n```rust\nfn main() {}\n```\n\n- a\n- b');
    expect(html).toContain('<h1>Title</h1>');
    expect(html).toContain('<strong>bold</strong>');
    expect(html).toContain('<code>code</code>');
    expect(html).toContain('<pre><code class="language-rust">fn main() {}');
    expect(html).toContain('<li>a</li>');
  });

  it('shows raw HTML as text', () => {
    const el = document.createElement('div');
    renderMarkdown(el, 'Hi <img src=x onerror=alert(1)> <script>alert(2)</script>\n\n<div onclick="x()">block</div>');
    expect(el.querySelector('img, script, div')).toBeNull();
    expect(el.textContent).toContain('<img src=x onerror=alert(1)>');
  });

  it('shows images as their alt text, so nothing is fetched', () => {
    const el = document.createElement('div');
    renderMarkdown(el, '![a diagram](https://example.com/x.png)');
    expect(el.querySelector('img')).toBeNull();
    expect(el.textContent).toContain('a diagram');
  });

  it('keeps only http, https and mailto links, marked for copying', () => {
    const el = document.createElement('div');
    renderMarkdown(el, '[ok](https://example.com/a) [bad](javascript:alert(1)) [file](file:///etc/passwd)');
    const links = [...el.querySelectorAll('a')];
    expect(links.map((a) => a.getAttribute('href'))).toEqual(['https://example.com/a']);
    expect(links[0]!.hasAttribute('data-kk-link')).toBe(true);
    expect(el.textContent).toContain('bad');
    expect(safeHref('mailto:a@b.c')).toBe('mailto:a@b.c');
    expect(safeHref('notaurl')).toBeUndefined();
  });

  it('escapes attribute-breaking characters in links', () => {
    const html = markdownToHtml('[x](https://example.com/"onmouseover="alert(1))');
    expect(html).not.toContain('" onmouseover');
  });
});
