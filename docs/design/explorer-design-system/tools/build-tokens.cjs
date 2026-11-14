#!/usr/bin/env node
/*
 * tokens.json（唯一の編集元）から以下を生成する。依存パッケージなし。
 *   - tokens.css
 *   - index.html の <style id="ex-tokens"> と <script id="ex-token-data">（表示用データ＋コントラスト計算結果）
 *
 *   node tools/build-tokens.cjs          生成して書き込む
 *   node tools/build-tokens.cjs --check  書き込まずにドリフトを検査（差分があれば exit 1）
 */
'use strict';
const fs = require('fs');
const path = require('path');

const ROOT = path.resolve(__dirname, '..');
const SRC = path.join(ROOT, 'tokens.json');
const CSS_OUT = path.join(ROOT, 'tokens.css');
const HTML = path.join(ROOT, 'index.html');
const PREFIX = '--ex-';

const tokens = JSON.parse(fs.readFileSync(SRC, 'utf8'));
const cssName = (id) => PREFIX + id.replace(/\./g, '-');

const THEMED = ['color', 'shadow', 'placeholderIcon'];
const STATIC = ['font', 'space', 'size', 'radius', 'line', 'motion'];

/* ---------- 色計算（WCAG 2.x 相対輝度） ---------- */
function parseColor(v) {
  v = String(v).trim().toLowerCase();
  if (v === 'transparent') return [0, 0, 0, 0];
  let m = v.match(/^#([0-9a-f]{3}|[0-9a-f]{6})$/);
  if (m) {
    let h = m[1];
    if (h.length === 3) h = h.split('').map((c) => c + c).join('');
    return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16), 1];
  }
  m = v.match(/^rgba?\(([^)]+)\)$/);
  if (m) {
    const p = m[1].split(',').map((s) => parseFloat(s));
    return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1];
  }
  return null;
}
const over = (top, base) => {
  const a = top[3];
  return [0, 1, 2].map((i) => top[i] * a + base[i] * (1 - a)).concat(1);
};
const lum = (c) => {
  const f = (x) => { x /= 255; return x <= 0.03928 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4); };
  return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]);
};
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
const hex = (c) => '#' + c.slice(0, 3).map((x) => Math.round(x).toString(16).padStart(2, '0')).join('');

function themedValue(id, theme, override) {
  if (override && override.token === id && override[theme]) return override[theme];
  for (const g of THEMED) if (tokens[g] && tokens[g][id]) return tokens[g][id][theme];
  throw new Error('unknown token in contrast pair: ' + id);
}
function measure(p, theme, override) {
  // 背景: over（下から上へ）を順に合成。最下層が不透明でなければ surface.canvas を敷く
  const layers = [...(p.over || []).slice().reverse(), p.bg].map((id) => parseColor(themedValue(id, theme, override)));
  let base = parseColor(themedValue('surface.canvas', theme));
  for (const l of layers) base = over(l, base);
  let fgRaw = themedValue(p.fg, theme, override);
  if (fgRaw === 'currentColor') fgRaw = themedValue(p.fgFallback || 'text.primary', theme, override);
  const fg = over(parseColor(fgRaw), base);
  return { r: ratio(fg, base), fg: hex(fg), bg: hex(base) };
}
function contrastResults() {
  return tokens.contrast.map((p) => {
    const out = { id: p.id, label: p.label, fg: p.fg, bg: p.bg, over: p.over || [], kind: p.kind, note: p.note || '', fix: p.fix || null };
    const target = p.kind === 'text' ? 4.5 : p.kind === 'nontext' ? 3 : 0;
    for (const theme of ['light', 'dark']) {
      const m = measure(p, theme);
      out[theme] = { ratio: Math.round(m.r * 100) / 100, fg: m.fg, bg: m.bg, target, pass: target === 0 ? null : m.r >= target };
      if (p.fix && p.fix[theme]) out[theme].fixed = Math.round(measure(p, theme, p.fix).r * 100) / 100;
    }
    return out;
  });
}

/* ---------- CSS 生成 ---------- */
function decls(groupNames, pick, indent = '  ') {
  const lines = [];
  for (const g of groupNames) {
    const group = tokens[g];
    if (!group) continue;
    lines.push(`${indent}/* ${g} */`);
    for (const [id, t] of Object.entries(group)) {
      const v = pick(t);
      if (v !== undefined) lines.push(`${indent}${cssName(id)}: ${v};`);
    }
  }
  return lines.join('\n');
}

function buildCss() {
  const head = `/*
 * Prism Canopy Compact — Explorer design tokens
 * GENERATED FILE. 直接編集しない。編集元: tokens.json
 * 再生成: node docs/design/explorer-design-system/tools/build-tokens.cjs
 * テーマ: <html data-theme="light|dark">。属性なしはOS設定（prefers-color-scheme）に従う。
 *         任意の要素に data-theme を付けると、その中だけテーマを固定できる。
 * 密度:   data-density="compact" で行高24px。
 */`;
  const light = decls(THEMED, (t) => t.light);
  const dark = decls(THEMED, (t) => t.dark);
  const darkIndented = decls(THEMED, (t) => t.dark, '    ');
  const stat = decls(STATIC, (t) => t.value);
  const mac = Object.entries(tokens.placeholderIcon).filter(([, t]) => t.mac).map(([id, t]) => `  ${cssName(id)}: ${t.mac};`).join('\n');
  const compact = Object.entries(tokens.size).filter(([, t]) => t.compact).map(([id, t]) => `  ${cssName(id)}: ${t.compact};`).join('\n');
  const narrow = Object.entries(tokens.size).filter(([, t]) => t.narrow).map(([id, t]) => `    ${cssName(id)}: ${t.narrow};`).join('\n');
  return `${head}
:root {
${stat}
}

:root,
[data-theme="light"] {
  color-scheme: light;
${light}
}

[data-theme="dark"] {
  color-scheme: dark;
${dark}
}

@media (prefers-color-scheme: dark) {
  :root:not([data-theme]) {
    color-scheme: dark;
${darkIndented}
  }
}

/* 仮素材アイコンの macOS 風切替（検証用。本番はOS取得アイコンをそのまま使う） */
[data-icons="mac"] {
${mac}
}

[data-density="compact"] {
${compact}
}

@media (max-width: 1140px) {
  :root {
${narrow}
  }
}
`;
}

function buildData(css) {
  const groups = {};
  for (const g of [...THEMED, ...STATIC]) {
    groups[g] = Object.entries(tokens[g] || {}).map(([id, t]) => ({ id, css: cssName(id), ...t }));
  }
  return { meta: tokens.$meta, groups, contrast: contrastResults(), legacyUnused: tokens.legacyUnused, cssBytes: Buffer.byteLength(css) };
}

function inject(html, css, data) {
  const styleRe = /(<style id="ex-tokens">)[\s\S]*?(<\/style>)/;
  const dataRe = /(<script type="application\/json" id="ex-token-data">)[\s\S]*?(<\/script>)/;
  if (!styleRe.test(html) || !dataRe.test(html)) throw new Error('index.html に生成マーカー（#ex-tokens / #ex-token-data）が見つかりません');
  const json = JSON.stringify(data).replace(/</g, '\\u003c');
  return html.replace(styleRe, (_, a, b) => `${a}\n${css}${b}`).replace(dataRe, (_, a, b) => `${a}${json}${b}`);
}

const css = buildCss();
const data = buildData(css);
const htmlNow = fs.readFileSync(HTML, 'utf8');
const htmlNext = inject(htmlNow, css, data);
const cssNow = fs.existsSync(CSS_OUT) ? fs.readFileSync(CSS_OUT, 'utf8') : '';

if (process.argv.includes('--check')) {
  const drift = [];
  if (cssNow !== css) drift.push('tokens.css');
  if (htmlNow !== htmlNext) drift.push('index.html（埋め込みトークン）');
  if (drift.length) { console.error('DRIFT: ' + drift.join(', ') + ' が tokens.json と一致しません。build-tokens.cjs を実行してください。'); process.exit(1); }
  console.log('OK: tokens.css と index.html の埋め込みは tokens.json と一致');
} else {
  fs.writeFileSync(CSS_OUT, css);
  fs.writeFileSync(HTML, htmlNext);
  const fails = data.contrast.flatMap((c) => ['light', 'dark'].filter((t) => c[t].pass === false).map((t) => `${c.id}(${t}) ${c[t].ratio}:1 < ${c[t].target}`));
  console.log(`wrote tokens.css (${data.cssBytes} B) and injected into index.html`);
  console.log(fails.length ? 'コントラスト未達: ' + fails.join(' / ') : 'コントラスト: すべて目標以上');
}
