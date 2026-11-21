// Сценарий «shared-хостинг»: index.html (площадка) + vladx-llm.php + config.php
// под `php -S`, вместо Polza — поддельный сервер. Проверяет, что .vlhmn
// переводится в браузере через PHP, а ключ остаётся на сервере.
//   node tests/browser/shared-hosting.mjs      (нужны php и собранный site/playground.html)
import { createRequire } from 'node:module';
import { spawn } from 'node:child_process';
import http from 'node:http';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
let chromium;
try { ({ chromium } = require('playwright')); } catch { ({ chromium } = require('playwright-core')); }

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const KEY = 'test-key-' + Math.random().toString(36).slice(2);

// ── поддельная Polza ──
const program = (body, confidence, questions) => JSON.stringify({ confidence, questions, program: { kind: 'Program', schema_version: 1, lang: 'ru', body } });
const SHOP = program([
  { kind: 'Declare', name: 'покупки', mutable: false, value: { kind: 'ListLit', items: ['хлеб', 'сыр'].map(v => ({ kind: 'StringLit', value: v, parts: null })) } },
  { kind: 'ForEach', item: 'вещь', iter: { kind: 'Identifier', name: 'покупки' }, body: { kind: 'Block', body: [
    { kind: 'Say', value: { kind: 'StringLit', value: '- {вещь}', parts: [{ kind: 'Text', value: '- ' }, { kind: 'Hole', expr: { kind: 'Identifier', name: 'вещь' } }] } }] } },
], 0.96, []);
const UNSURE = program([{ kind: 'Say', value: { kind: 'NumberLit', value: 42 } }], 0.4, ['Какое число вывести?']);
let polzaCalls = 0, sawKey = true;
const polza = http.createServer((req, res) => {
  let b = ''; req.on('data', c => b += c); req.on('end', () => {
    polzaCalls++;
    if (req.headers.authorization !== 'Bearer ' + KEY) sawKey = false;
    const user = JSON.parse(b).messages[1].content;
    const content = user.includes('покуп') ? SHOP : UNSURE;
    res.setHeader('Content-Type', 'application/json');
    res.end(JSON.stringify({ choices: [{ message: { role: 'assistant', content }, finish_reason: 'stop' }] }));
  });
}).listen(0, '127.0.0.1');
await new Promise(r => polza.on('listening', r));

// ── «хостинг» ──
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vladx-shared-'));
fs.copyFileSync(path.join(ROOT, 'site/playground.html'), path.join(dir, 'index.html'));
fs.copyFileSync(path.join(ROOT, 'deploy/shared/vladx-llm.php'), path.join(dir, 'vladx-llm.php'));
fs.writeFileSync(path.join(dir, 'config.php'), `<?php return ['polza_api_key' => '${KEY}', 'endpoint' => 'http://127.0.0.1:${polza.address().port}/v1/chat/completions', 'model' => 'anthropic/claude-haiku-5.5'];`);
const port = 18000 + Math.floor(Math.random() * 1000);
const php = spawn('php', ['-S', `127.0.0.1:${port}`, '-t', dir], { stdio: 'ignore', env: { ...process.env, http_proxy: '', HTTP_PROXY: '' } });
for (let i = 0; i < 50; i++) { try { await fetch(`http://127.0.0.1:${port}/vladx-llm.php`); break; } catch { await new Promise(r => setTimeout(r, 100)); } }

const browser = await chromium.launch();
const page = await (await browser.newContext()).newPage();
const errors = [], browserRequests = [];
page.on('pageerror', e => errors.push(String(e)));
page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
page.on('request', r => browserRequests.push(r));

let passed = 0, failed = 0;
const check = (n, name, ok, detail = '') => {
  if (ok) { passed++; console.log(`  ok   ${n}. ${name}`); } else { failed++; console.log(`  FAIL ${n}. ${name}\n       ${String(detail).replace(/\n/g, '\n       ')}`); }
};
const idle = () => page.waitForFunction(() => !/выполняется|переводит/.test(document.getElementById('status').textContent), null, { timeout: 60000 });
async function run(text) { await idle(); await page.fill('#code', text); await page.click('#run'); await page.waitForTimeout(50); await idle(); return page.innerText('#output'); }

try {
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.waitForFunction(() => !document.getElementById('run').disabled, null, { timeout: 30000 });
  await idle();
  await page.waitForFunction(() => /vladx-llm\.php/.test(document.getElementById('fmt').title), null, { timeout: 10000 });
  check(1, 'площадка сама нашла vladx-llm.php рядом с собой', true);

  await page.selectOption('#fmt', 'vlhmn');
  let out = await run('Сделай список покупок: хлеб и сыр. Выведи каждый пункт с тире.');
  check(2, '.vlhmn переведён через PHP и исполнен в браузере', out.includes('ИИ понял так') && out.includes('- хлеб') && out.includes('- сыр') && polzaCalls === 1, out);
  check(3, 'Polza получила ключ от PHP', sawKey && polzaCalls === 1);

  out = await run('Сделай список покупок: хлеб и сыр. Выведи каждый пункт с тире.');
  check(4, 'повторный запуск — из кэша, без запроса к ИИ', out.includes('из кэша') && polzaCalls === 1, out);

  out = await run('Выведи что-нибудь, неясно что.');
  const low = await page.isVisible('#lowconf');
  check(5, 'низкая уверенность → вопросы и панель «Всё равно запустить»', low && out.includes('Какое число вывести?') && !/(^|\n)42(\n|$)/.test(out), out);
  await page.click('#runanyway'); await page.waitForTimeout(50); await idle();
  out = await page.innerText('#output');
  check(6, '«Всё равно запустить» исполняет перевод', /(^|\n)42(\n|$)/.test(out), out);

  const leaked = browserRequests.some(r => JSON.stringify(r.headers()).includes(KEY) || (r.postData() || '').includes(KEY));
  const html = await page.content();
  check(7, 'ключ не попадает в браузер (ни в запросы, ни в страницу)', !leaked && !html.includes(KEY));
  const direct = browserRequests.filter(r => r.url().includes(String(polza.address().port)));
  check(8, 'браузер не ходит в Polza напрямую', direct.length === 0, direct.map(r => r.url()).join('\n'));
  check(9, 'в консоли нет ошибок', errors.length === 0, errors.join('\n'));
} finally {
  await browser.close(); php.kill(); polza.close();
  fs.rmSync(dir, { recursive: true, force: true });
}
console.log(`\n${passed} прошло, ${failed} упало`);
process.exit(failed ? 1 : 0);
