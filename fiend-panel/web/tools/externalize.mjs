import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';

const path = new URL('../build/index.html', import.meta.url);
let html = await readFile(path, 'utf8');
const scripts = [...html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/g)];
for (const [tag, attributes, source] of scripts) {
  if (/\bsrc\s*=/.test(attributes)) continue;
  if (attributes.trim()) throw new Error('Unexpected inline script attributes');
  const hash = createHash('sha256').update(source).digest('hex').slice(0, 16);
  const name = `bootstrap.${hash}.js`;
  await writeFile(new URL(`../build/${name}`, import.meta.url), source);
  html = html.replace(tag, `<script src="/${name}"></script>`);
}
if (/<script\b(?![^>]*\bsrc=)|<style\b|\sstyle=|\son\w+=/i.test(html)) {
  throw new Error('The SPA needs inline code; strict CSP would block it');
}
await writeFile(path, html);
