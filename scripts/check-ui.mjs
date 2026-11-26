// Confere a interface do app de desktop (crates/gui/ui) sem precisar de navegador nem de Node extra:
//  - app.js e i18n.js têm sintaxe válida;
//  - en e es têm exatamente as mesmas chaves de texto que pt;
//  - toda chave usada no HTML (data-i18n) ou no JS (t('...')) existe;
//  - todo $('#id') do JS existe no HTML e não há ids repetidos.
// Uso: node scripts/check-ui.mjs
import { readFileSync } from 'node:fs';
import { Script, createContext, runInContext } from 'node:vm';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const dir = join(dirname(fileURLToPath(import.meta.url)), '..', 'crates', 'gui', 'ui');
const read = (f) => readFileSync(join(dir, f), 'utf8');
const problems = [];
const fail = (msg) => problems.push(msg);

const js = read('app.js');
const i18nSrc = read('i18n.js');
const html = read('index.html');

for (const [name, src] of [['app.js', js], ['i18n.js', i18nSrc]]) {
  try { new Script(src, { filename: name }); } catch (e) { fail(`${name}: sintaxe inválida: ${e.message}`); }
}

let I18N = null;
try {
  const ctx = createContext({});
  I18N = runInContext(`${i18nSrc}\n;I18N`, ctx);
} catch (e) {
  fail(`i18n.js não definiu I18N: ${e.message}`);
}

if (I18N) {
  const base = Object.keys(I18N.pt || {});
  for (const lang of Object.keys(I18N)) {
    if (lang === 'pt') continue;
    const have = new Set(Object.keys(I18N[lang]));
    for (const k of base) if (!have.has(k)) fail(`i18n: falta "${k}" em ${lang}`);
    for (const k of have) if (!(k in I18N.pt)) fail(`i18n: "${k}" existe em ${lang} mas não em pt`);
  }
  const known = new Set(base);
  const usedHtml = [...html.matchAll(/data-i18n(?:-title|-aria)?="([^"]+)"/g)].map((m) => m[1]);
  const usedJs = [...js.matchAll(/\bt\('([A-Za-z0-9_.]+[A-Za-z0-9_])'/g)].map((m) => m[1]);
  for (const k of new Set([...usedHtml, ...usedJs])) if (!known.has(k)) fail(`i18n: chave usada e não definida: "${k}"`);
}

const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map((m) => m[1]);
const idSet = new Set(ids);
for (const id of ids.filter((v, i, a) => a.indexOf(v) !== i)) fail(`html: id repetido "${id}"`);
for (const id of new Set([...js.matchAll(/\$\('#([A-Za-z0-9_-]+)'\)/g)].map((m) => m[1]))) {
  if (!idSet.has(id)) fail(`app.js usa #${id}, que não existe no HTML`);
}

if (problems.length) {
  console.error(`check-ui: ${problems.length} problema(s)`);
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}
console.log('check-ui: ok');
