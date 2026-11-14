#!/usr/bin/env node
/*
 * デザインシステム成果物の検証（Headless Chrome / file:// / HTTPサーバー不要）
 *   node docs/design/explorer-design-system/tools/verify.cjs
 * 環境変数 PLAYWRIGHT_PATH / CHROME_PATH で場所を上書きできる。
 * 結果: screenshots/verify-report.json と screenshots/*.png。失敗があれば exit 1。
 */
'use strict';
const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');
const { pathToFileURL } = require('url');

const PW = process.env.PLAYWRIGHT_PATH || 'playwright';
const CHROME = process.env.CHROME_PATH || 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const { chromium } = require(PW);

const ROOT = path.resolve(__dirname, '..');
const SHOTS = path.join(ROOT, 'screenshots');
const MOCK_DIR = path.resolve(ROOT, '../explorer-directions/argon');
const INDEX = pathToFileURL(path.join(ROOT, 'index.html')).href;
const tokens = JSON.parse(fs.readFileSync(path.join(ROOT, 'tokens.json'), 'utf8'));
fs.mkdirSync(SHOTS, { recursive: true });

const report = { date: new Date().toISOString(), checks: [], mockDrift: [], contrastRendered: [], screenshots: [] };
let failures = 0;
function check(name, ok, detail) {
  report.checks.push({ name, ok: !!ok, detail: detail === undefined ? '' : detail });
  if (!ok) failures++;
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail !== undefined && !ok ? ' — ' + JSON.stringify(detail) : ''}`);
}
const settle = (p, ms = 450) => p.evaluate((ms) => new Promise((r) => setTimeout(() => requestAnimationFrame(() => requestAnimationFrame(r)), ms)), ms);
async function shot(p, name, opts = {}) {
  const file = path.join(SHOTS, name);
  if (opts.locator) await opts.locator.screenshot({ path: file });
  else await p.screenshot({ path: file, fullPage: !!opts.fullPage });
  report.screenshots.push(name);
}
const themed = () => ['color', 'shadow', 'placeholderIcon'].flatMap((g) => Object.entries(tokens[g]).map(([id, t]) => ({ id, ...t })));
const statics = () => ['font', 'space', 'size', 'radius', 'line', 'motion'].flatMap((g) => Object.entries(tokens[g]).map(([id, t]) => ({ id, ...t })));
const cssName = (id) => '--ex-' + id.replace(/\./g, '-');

(async () => {
  /* 1. 生成物のドリフト */
  try { execFileSync(process.execPath, [path.join(__dirname, 'build-tokens.cjs'), '--check'], { stdio: 'pipe' }); check('tokens.css / index.html 埋め込みが tokens.json と一致（build-tokens --check）', true); }
  catch (e) { check('tokens.css / index.html 埋め込みが tokens.json と一致（build-tokens --check）', false, String(e.stderr || e.message)); }
  const html = fs.readFileSync(path.join(ROOT, 'index.html'), 'utf8');
  const embedded = (html.match(/<style id="ex-tokens">\n([\s\S]*?)<\/style>/) || [])[1];
  check('埋め込み CSS が tokens.css とバイト一致', embedded === fs.readFileSync(path.join(ROOT, 'tokens.css'), 'utf8'));
  check('index.html に外部リソース参照なし（http/https の src/href なし）', !/(src|href)=["']https?:/i.test(html));

  const browser = await chromium.launch({ executablePath: CHROME, headless: true });
  try {
    /* 2. 採用済みモックとの値一致（モックの実描画から読む） */
    for (const [theme, file] of [['light', 'concept-1.html'], ['dark', 'concept-1-dark.html']]) {
      const p = await browser.newPage({ viewport: { width: 1280, height: 800 } });
      await p.addInitScript(() => { try { localStorage.clear(); } catch (e) {} });
      await p.goto(pathToFileURL(path.join(MOCK_DIR, file)).href);
      await settle(p, 300);
      const probes = [...themed(), ...statics()].filter((t) => t.probe).map((t) => ({ id: t.id, probe: t.probe, expected: t.probe.prop === 'height' || t.probe.prop === 'width' || /radius/.test(t.probe.prop || '') ? t.value : (t[theme] !== undefined ? t[theme] : t.value) }));
      const res = await p.evaluate((probes) => {
        const norm = (v, kind) => {
          const el = document.createElement('div'); document.body.appendChild(el);
          if (kind === 'shadow') el.style.boxShadow = v; else el.style.color = v;
          const out = kind === 'shadow' ? getComputedStyle(el).boxShadow : getComputedStyle(el).color; el.remove(); return out;
        };
        return probes.map(({ id, probe, expected }) => {
          let actual;
          if (probe.var) actual = getComputedStyle(document.documentElement).getPropertyValue(probe.var).trim();
          else { const el = document.querySelector(probe.sel); if (!el) return { id, missing: probe.sel }; actual = getComputedStyle(el)[probe.prop.replace(/-([a-z])/g, (_, c) => c.toUpperCase())]; }
          const isLen = /px$/.test(expected);
          const isShadow = id.startsWith('shadow.');
          const a = isLen ? parseFloat(actual) : norm(actual, isShadow ? 'shadow' : ''), e = isLen ? parseFloat(expected) : norm(expected, isShadow ? 'shadow' : '');
          return { id, expected, actual, ok: isLen ? Math.abs(a - e) < 0.6 : a === e };
        });
      }, probes);
      for (const r of res) report.mockDrift.push({ theme, ...r });
      const bad = res.filter((r) => !r.ok);
      check(`モック ${file} の実描画とトークン値が一致（${res.length} 項目）`, bad.length === 0, bad);
      await p.close();
    }

    /* 3. リファレンス：各幅・各テーマ */
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const errors = [];
    const open = async (vw, vh, theme, init) => {
      const p = await ctx.newPage();
      await p.setViewportSize({ width: vw, height: vh });
      p.on('pageerror', (e) => errors.push(`${vw}/${theme}: ${e.message}`));
      p.on('console', (m) => { if (m.type() === 'error') errors.push(`${vw}/${theme}: ${m.text()}`); });
      await p.addInitScript((t) => { try { localStorage.setItem('ex-ds-theme', t); } catch (e) {} }, theme);
      if (init) await p.addInitScript(init);
      await p.goto(INDEX);
      await settle(p, 300);
      return p;
    };
    const overflow = (p) => p.evaluate(() => {
      const over = document.documentElement.scrollWidth - innerWidth;
      const offenders = over > 0 ? [...document.querySelectorAll('body *')].filter((el) => el.getBoundingClientRect().right > innerWidth + 1 && !el.closest('.demo-body,.tbl-wrap,.pair-col,.ex-tabs,.doc-toc')).slice(0, 5).map((el) => el.className || el.tagName) : [];
      return { over, offenders };
    });

    for (const [vw, vh] of [[1280, 800], [1100, 700], [390, 844]]) {
      for (const theme of ['light', 'dark']) {
        const p = await open(vw, vh, theme);
        const o = await overflow(p);
        check(`${vw}px ${theme}: 横はみ出しなし`, o.over <= 0, o);
        const bg = await p.evaluate(() => getComputedStyle(document.body).backgroundColor);
        check(`${vw}px ${theme}: 地の色が ${theme} の surface.canvas`, bg === (theme === 'light' ? 'rgb(245, 247, 250)' : 'rgb(25, 25, 25)'), bg);
        await shot(p, `reference-${vw}-${theme}.png`);
        if (vw === 1280) {
          for (const id of ['colors', 'contrast', 'c-row', 'p-compose']) {
            await p.evaluate((id) => document.getElementById(id).scrollIntoView(), id);
            await settle(p, 200);
            await shot(p, `reference-${vw}-${theme}-${id}.png`);
          }
          await shot(p, `compose-${theme}.png`, { locator: p.locator('.compose') });
          await shot(p, `states-rows-${theme}.png`, { locator: p.locator('#states .demo').first() });
          await shot(p, `tabs-${theme}.png`, { locator: p.locator('#c-tabs .demo').first() });
          await shot(p, `sidebar-${theme}.png`, { locator: p.locator('#c-sidebar .demo').first() });
          await shot(p, `menu-${theme}.png`, { locator: p.locator('#c-menu .demo').first() });
        }
        await p.close();
      }
    }

    /* 4. トークンの整合（ブラウザーの計算値 = tokens.json） */
    {
      const p = await open(1280, 800, 'light');
      for (const theme of ['light', 'dark']) {
        await p.click(`[data-theme-opt="${theme}"]`);
        await settle(p);
        const list = themed().map((t) => ({ css: cssName(t.id), v: t[theme] })).concat(statics().map((t) => ({ css: cssName(t.id), v: t.value })));
        const bad = await p.evaluate((list) => list.filter(({ css, v }) => getComputedStyle(document.documentElement).getPropertyValue(css).trim() !== v).map(({ css, v }) => ({ css, v, got: getComputedStyle(document.documentElement).getPropertyValue(css).trim() })), list);
        check(`${theme}: CSS 変数の計算値が tokens.json と一致（${list.length} 項目）`, bad.length === 0, bad.slice(0, 5));
        const shown = await p.evaluate(() => [...document.querySelectorAll('#color-groups .sw')].map((el) => ({ id: el.dataset.token, l: el.querySelector('.sw-chip.l span').textContent, d: el.querySelector('.sw-chip.d span').textContent })));
        const mism = shown.filter((s) => tokens.color[s.id].light !== s.l || tokens.color[s.id].dark !== s.d);
        if (theme === 'light') check(`HTML の色カード表示値が tokens.json と一致（${shown.length} 枚）`, mism.length === 0 && shown.length === Object.keys(tokens.color).length, mism);
      }

      /* 5. 実描画コントラスト（DOM の文字色と、祖先の背景を合成した実効背景から計算） */
      const samples = [
        ['row-name', '#live-rows .ex-row:not(.sel):not(.hid) .nm', 'color'],
        ['row-meta', '#live-rows .ex-row:not(.sel) .c-mod', 'color'],
        ['sel-name', '#states .demo-body > .states .ex-rows.focused .ex-row.sel:not(.is-kbd):not(.hid) .c-name', 'color'],
        ['sel-meta', '#states .demo-body > .states .ex-rows.focused .ex-row.sel:not(.is-kbd):not(.hid) .c-mod', 'color'],
        ['sel-inactive-name', '#states .demo-body > .states .ex-rows:not(.focused) .ex-row.sel .c-name', 'color'],
        ['header', '#live-lh .ex-hbtn:not([data-active])', 'color'],
        ['header-sorted', '#live-lh .ex-hbtn[data-active]', 'color'],
        ['nav-current', '#live-sidebar .ex-place[aria-current="page"]', 'color'],
        ['side-heading', '#live-sidebar .ex-side-h', 'color'],
        ['drive-caption', '#live-sidebar .ex-place.drive:not([aria-current]) .ex-drive-free', 'color'],
        ['tab-active', '#live-tabs .ex-tab.active .ex-tab-select', 'color'],
        ['tab-inactive', '#live-tabs .ex-tab:not(.active) .ex-tab-select', 'color'],
        ['crumb-current', '#c-address .demo-body > .states .ex-crumb[aria-current="page"]:not(.is-focus)', 'color'],
        ['button', '#c-button .demo-body > .states .ex-btn:not(.primary):not(.is-hover):not(.is-focus):not(.is-disabled)', 'color'],
        ['button-primary', '#c-button .demo-body > .states .ex-btn.primary:not(.is-hover):not(.is-focus):not(.is-disabled)', 'color'],
        ['badge', '#c-inspector .demo-body > .states .ex-badge', 'color'],
        ['inspector-label', '#c-inspector .demo-body > .states .ex-props dt', 'color'],
        ['status-selected', '#live-sel', 'color'],
        ['menu-current', '#c-menu .demo-body > .states .ex-mi.cur', 'color'],
        ['menu-kbd', '#live-menu .ex-mi:not(.cur) .kbd', 'color'],
      ];
      const computed = Object.fromEntries(JSON.parse(html.match(/id="ex-token-data">([\s\S]*?)<\/script>/)[1]).contrast.map((c) => [c.id, c]));
      for (const theme of ['light', 'dark']) {
        await p.click(`[data-theme-opt="${theme}"]`);
        await p.locator('#live-rows').evaluate((el) => el.blur());
        await settle(p);
        const res = await p.evaluate((samples) => {
          const parse = (s) => { const m = s.match(/rgba?\(([^)]+)\)/); if (!m) return [0, 0, 0, 0]; const v = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [v[0], v[1], v[2], v.length > 3 ? v[3] : 1]; };
          const over = (t, b) => [0, 1, 2].map((i) => t[i] * t[3] + b[i] * (1 - t[3])).concat(1);
          const lum = (c) => { const f = (x) => { x /= 255; return x <= 0.03928 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4); }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
          const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
          return samples.map(([id, sel]) => {
            const el = document.querySelector(sel);
            if (!el) return { id, missing: sel };
            const layers = [];
            for (let n = el; n; n = n.parentElement) { const c = parse(getComputedStyle(n).backgroundColor); if (c[3] > 0) layers.push(c); if (c[3] >= 1) break; }
            let bg = [255, 255, 255, 1];
            for (const l of layers.reverse()) bg = over(l, bg);
            const fg = over(parse(getComputedStyle(el).color), bg);
            return { id, ratio: Math.round(ratio(fg, bg) * 100) / 100 };
          });
        }, samples);
        for (const r of res) {
          const c = computed[r.id];
          const exp = c && c[theme].ratio;
          report.contrastRendered.push({ theme, ...r, computed: exp });
        }
        const bad = res.filter((r) => r.missing || Math.abs(r.ratio - computed[r.id][theme].ratio) > 0.06);
        check(`${theme}: 実描画のコントラスト比が計算値と一致（${res.length} 組、許容差 0.06）`, bad.length === 0, bad.map((r) => ({ ...r, computed: computed[r.id] && computed[r.id][theme].ratio })));
      }
      await p.close();
    }

    /* 6. 操作：目次・テーマ・並列・コピー・各展示 */
    {
      const p = await open(1280, 800, 'light');
      // 目次の絞り込み
      await p.fill('#toc-filter', '選択');
      let vis = await p.$$eval('[data-toc-link]', (as) => as.filter((a) => !a.hidden).map((a) => a.textContent));
      check('目次の絞り込み「選択」で関連項目のみ表示', vis.includes('ファイル行') && vis.includes('状態と優先順位') && !vis.includes('タイポグラフィ'), vis);
      await p.fill('#toc-filter', 'zzzz該当なし');
      check('目次の絞り込み 0 件で「該当なし」表示', await p.isVisible('#toc-empty'));
      await p.click('#toc-filter-clear');
      vis = await p.$$eval('[data-toc-link]', (as) => as.filter((a) => !a.hidden).length);
      check('絞り込み解除で全項目に戻る', vis === (await p.$$eval('.doc-sec', (s) => s.length)), vis);
      await p.click('[data-toc-link="c-search"]');
      await settle(p, 600);
      const top = await p.$eval('#c-search', (el) => el.getBoundingClientRect().top);
      const cur = await p.getAttribute('[data-toc-link="c-search"]', 'aria-current');
      check('目次リンクで該当節へ移動し現在位置を強調', top > 0 && top < 120 && cur === 'true', { top, cur });

      // テーマ切替（遷移完了を待ってから判定・撮影）
      await p.click('[data-theme-opt="dark"]'); await settle(p);
      check('テーマ: ダークに切替', (await p.evaluate(() => [document.documentElement.dataset.theme, getComputedStyle(document.body).backgroundColor])).join() === 'dark,rgb(25, 25, 25)');
      await p.emulateMedia({ colorScheme: 'dark' });
      await p.click('[data-theme-opt="system"]'); await settle(p);
      const sys = await p.evaluate(() => [document.documentElement.hasAttribute('data-theme'), getComputedStyle(document.body).backgroundColor, localStorage.getItem('ex-ds-theme')]);
      check('テーマ: システム（OS ダーク）で data-theme を外しダーク表示・保存', sys[0] === false && sys[1] === 'rgb(25, 25, 25)' && sys[2] === 'system', sys);
      await p.emulateMedia({ colorScheme: 'light' }); await settle(p);
      check('テーマ: システム（OS ライト）でライト表示', (await p.evaluate(() => getComputedStyle(document.body).backgroundColor)) === 'rgb(245, 247, 250)');
      await p.click('[data-theme-opt="light"]'); await settle(p);

      // 明暗並列
      await p.click('#pair-toggle'); await settle(p);
      const pair = await p.evaluate(() => {
        const cols = [...document.querySelectorAll('.pair-col[data-theme="dark"]')];
        const sel = cols[0] && cols[0].closest('.demo').querySelector('.pair-col[data-theme="dark"] .ex-rows.focused .ex-row.sel');
        return { count: cols.length, visible: cols.length && cols[0].offsetParent !== null, selBg: sel && getComputedStyle(sel).backgroundColor, inert: cols.every((c) => c.inert), dupIds: cols.some((c) => c.querySelector('[id]')) };
      });
      check('明暗並列: 状態見本がライト/ダークで並び、ダーク列はダークのトークンで描画（複製は操作不可・ID重複なし）', pair.count > 10 && pair.visible && pair.selBg === 'rgb(37, 52, 76)' && pair.inert && !pair.dupIds, pair);
      await p.evaluate(() => document.getElementById('c-row').scrollIntoView());
      await p.evaluate(() => window.scrollBy(0, 480));
      await settle(p, 200);
      await shot(p, 'reference-1280-pair-view.png');
      await p.click('#pair-toggle'); await settle(p, 200);

      // コピー成功経路（Clipboard API か execCommand）
      await ctx.grantPermissions(['clipboard-read', 'clipboard-write']).catch(() => {});
      const btn = p.locator('#color-groups .copy').first();
      await btn.scrollIntoViewIfNeeded();
      await btn.click();
      await p.waitForTimeout(200);
      const how = await btn.getAttribute('data-copied');
      check('コピー: 通常経路で成功しトースト表示', (how === 'clipboard' || how === 'execCommand') && await p.isVisible('#toast'), how);

      // 展示：タブ
      await p.click('#live-tab-add');
      check('タブ: 追加で 4 枚・新タブが現在', (await p.$$eval('#live-tabs .ex-tab', (t) => t.length)) === 4 && (await p.getAttribute('#live-tabs .ex-tab.active [data-tab]', 'data-tab')) === '4');
      await p.click('#live-tabs .ex-tab.active .ex-tab-select');
      await p.keyboard.press('Home');
      check('タブ: Home で先頭へ', (await p.getAttribute('#live-tabs [aria-selected="true"]', 'data-tab')) === '1');
      await p.keyboard.press('ArrowRight');
      check('タブ: → で次へ', (await p.getAttribute('#live-tabs [aria-selected="true"]', 'data-tab')) === '2');
      await p.keyboard.press('Delete');
      check('タブ: Delete で閉じて隣へ', (await p.$$eval('#live-tabs .ex-tab', (t) => t.length)) === 3 && (await p.getAttribute('#live-tabs [aria-selected="true"]', 'data-tab')) === '3');
      for (let i = 0; i < 5; i++) await p.click('#live-tab-add');
      const tabOv = await p.$eval('#live-tabs', (el) => [el.scrollWidth > el.clientWidth, el.scrollLeft > 0]);
      check('タブ: 多数で横スクロールし現在タブが見える位置へ', tabOv[0] && tabOv[1], tabOv);

      // 展示：検索
      await p.fill('#live-search', '仕様');
      check('検索: 入力で絞り込み・一致マーカー・クリア表示', (await p.$$eval('#live-search-rows .ex-row', (r) => r.length)) === 1 && await p.isVisible('#live-search-rows mark') && await p.isVisible('#live-search-clear'));
      await p.fill('#live-search', 'zzz');
      check('検索: 0 件表示（採用済み文言）', (await p.textContent('#live-search-rows')).includes('条件に一致する項目はありません'));
      await p.click('[data-clear-live-search]');
      check('検索: 「フィルターを解除」で復帰しクリア非表示', (await p.inputValue('#live-search')) === '' && !(await p.isVisible('#live-search-clear')));
      await p.fill('#live-search', '企画'); await p.press('#live-search', 'Escape');
      check('検索: Esc でクリア', (await p.inputValue('#live-search')) === '');

      // 展示：一覧
      const rows = p.locator('#live-rows .ex-row');
      await rows.nth(2).click();
      await rows.nth(4).click({ modifiers: ['Control'] });
      check('一覧: クリック＋Ctrl+クリックで 2 件選択・フォーカス中の塗り', (await p.$$eval('#live-rows .ex-row.sel', (r) => r.length)) === 2 && (await p.$eval('#live-rows .ex-row.sel', (el) => getComputedStyle(el).backgroundColor)) === 'rgb(232, 239, 252)');
      await rows.nth(6).click({ modifiers: ['Shift'] });
      check('一覧: Shift+クリックで範囲選択', (await p.$$eval('#live-rows .ex-row.sel', (r) => r.length)) === 3);
      await p.keyboard.press('ArrowDown');
      const ring = await p.$eval('#live-rows .ex-row.active', (el) => getComputedStyle(el).boxShadow);
      check('一覧: キー移動で現在行に 2px のフォーカスリング', /2px/.test(ring) && (await p.$eval('#live-rows', (el) => el.classList.contains('kbd'))), ring);
      await p.click('#c-row h2');
      await settle(p, 250);
      const inactive = await p.$eval('#live-rows .ex-row.sel', (el) => getComputedStyle(el).backgroundColor);
      check('一覧: フォーカスが外れると非アクティブ選択の塗り', inactive === 'rgba(148, 163, 184, 0.14)', inactive);
      await p.click('[data-density-opt="compact"]');
      check('一覧: コンパクトで行高 24px', Math.round((await rows.first().boundingBox()).height) === 24);
      await p.click('[data-density-opt="standard"]');
      check('一覧: 標準で行高 28px', Math.round((await rows.first().boundingBox()).height) === 28);
      await p.click('#live-lh [data-sort="size"]');
      await p.click('#live-lh [data-sort="size"]');
      check('一覧: 列見出しでソートし再押下で降順', (await p.getAttribute('#live-lh [data-sort="size"]', 'aria-sort')) === 'descending' && (await p.$$eval('#live-lh .ex-hbtn[data-active]', (b) => b.length)) === 1);
      await p.click('#live-view-grid');
      check('一覧: グリッド表示へ切替（ヘッダー非表示・タイル）', (await p.$$eval('#live-rows .ex-tile', (t) => t.length)) === 11 && !(await p.isVisible('#live-lh')));
      await p.click('#live-view-list');

      // 展示：メニュー
      await p.click('#live-menu-btn');
      check('メニュー: 開くと aria-expanded=true で先頭項目にフォーカス', (await p.getAttribute('#live-menu-btn', 'aria-expanded')) === 'true' && (await p.evaluate(() => document.activeElement.classList.contains('cur') && document.activeElement.closest('#live-menu') !== null)));
      await p.keyboard.press('ArrowDown'); await p.keyboard.press('ArrowDown'); await p.keyboard.press('ArrowDown');
      check('メニュー: ↓ で移動し無効項目を飛ばして先頭へ戻る', (await p.evaluate(() => document.activeElement.textContent.trim())) === '標準（28px）');
      await p.keyboard.press('ArrowDown'); await p.keyboard.press('Enter');
      check('メニュー: Enter で選択して閉じフォーカスをボタンへ戻す', (await p.isHidden('#live-menu')) && (await p.evaluate(() => document.activeElement.id)) === 'live-menu-btn');
      await p.click('#live-menu-btn'); await p.keyboard.press('Escape');
      check('メニュー: Esc で閉じる', await p.isHidden('#live-menu'));

      // 展示：ダイアログ
      await p.click('#live-dialog-open');
      check('ダイアログ: 開く', await p.$eval('#live-dialog', (d) => d.open));
      await p.keyboard.press('Escape');
      check('ダイアログ: Esc で閉じフォーカスを戻す', !(await p.$eval('#live-dialog', (d) => d.open)) && (await p.evaluate(() => document.activeElement.id)) === 'live-dialog-open');

      // キーボードフォーカスの可視化
      await p.evaluate(() => { document.activeElement && document.activeElement.blur(); window.scrollTo(0, 0); });
      await p.keyboard.press('Tab');
      const fo = await p.evaluate(() => { const s = getComputedStyle(document.activeElement); return [document.activeElement.className, s.outlineStyle, s.outlineWidth]; });
      check('Tab 操作でフォーカスリング（2px solid）が見える', fo[1] === 'solid' && fo[2] === '2px', fo);
      await p.close();
    }

    /* 7. コピー失敗時のフォールバック（Clipboard API と execCommand を失敗させる） */
    {
      const p = await open(1280, 800, 'light', () => {
        Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: () => Promise.reject(new Error('denied')) } });
        document.execCommand = () => false;
      });
      const btn = p.locator('#files [data-copy-from]');
      await btn.scrollIntoViewIfNeeded();
      await btn.click();
      await p.waitForTimeout(200);
      const st = await p.evaluate(() => { const ta = document.getElementById('copy-text'); return { open: document.getElementById('copy-dialog').open, value: ta.value, selected: ta.selectionStart === 0 && ta.selectionEnd === ta.value.length && document.activeElement === ta }; });
      const expected = await p.textContent('#cmd-pre');
      check('コピー失敗時: 手動コピー用ダイアログに全文を選択状態で表示', st.open && st.value === expected && st.selected, st);
      await settle(p, 200);
      await shot(p, 'reference-copy-fallback.png');
      await p.keyboard.press('Escape');
      check('コピー失敗時: Esc でダイアログを閉じる', !(await p.$eval('#copy-dialog', (d) => d.open)));
      await p.close();
    }

    /* 8. 狭幅 390px：目次ドロワーとサンプル */
    {
      const p = await open(390, 844, 'light');
      check('390px: 目次は初期非表示（ドロワー）', !(await p.isVisible('#toc')) || (await p.$eval('#toc', (el) => getComputedStyle(el).visibility)) === 'hidden');
      await p.click('#toc-toggle'); await settle(p, 300);
      const box = await p.locator('#toc').boundingBox();
      check('390px: 目次ボタンでドロワーが画面内に開き検索欄へフォーカス', box && box.x >= 0 && box.x + box.width <= 390 && (await p.evaluate(() => document.activeElement.id)) === 'toc-filter', box);
      await shot(p, 'reference-390-light-toc.png');
      await p.fill('#toc-filter', 'メニュー');
      await p.press('#toc-filter', 'Enter'); await settle(p, 500);
      const t = await p.$eval('#c-menu', (el) => el.getBoundingClientRect().top);
      check('390px: 絞り込み＋Enter で節へ移動しドロワーを閉じる', !(await p.evaluate(() => document.body.classList.contains('toc-open'))) && t >= 0 && t < 140, t);
      const o = await overflow(p);
      check('390px: 節移動後も横はみ出しなし', o.over <= 0, o);
      for (const id of ['c-row', 'contrast', 'c-tabs']) {
        await p.evaluate((id) => document.getElementById(id).scrollIntoView(), id); await settle(p, 200);
        await shot(p, `reference-390-light-${id}.png`);
      }
      const demoScroll = await p.$eval('#c-row .demo.live .demo-body', (el) => el.scrollWidth > el.clientWidth);
      check('390px: 幅の広いサンプルは枠内で横スクロール（ページは崩れない）', demoScroll);
      await p.close();
    }

    check('全ページでコンソールエラー・例外なし', errors.length === 0, errors);
    await ctx.close();
  } finally {
    await browser.close();
  }

  report.summary = { total: report.checks.length, failed: failures };
  fs.writeFileSync(path.join(SHOTS, 'verify-report.json'), JSON.stringify(report, null, 2) + '\n');
  console.log(`\n${failures ? 'FAILED' : 'ALL PASS'}: ${report.checks.length - failures}/${report.checks.length}  → screenshots/verify-report.json`);
  process.exit(failures ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(1); });
