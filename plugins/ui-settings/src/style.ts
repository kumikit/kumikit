/** Links the plugin's stylesheet once; the CSP forbids inline styles, so it is a file
 * next to the module. */
export function linkStyles(): () => void {
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = new URL(/* @vite-ignore */ './style.css', import.meta.url).href;
  document.head.append(link);
  return () => link.remove();
}
