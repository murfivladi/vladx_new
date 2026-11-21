// Сценарий «shared-хостинг»: index.php + playground.html + vladx-*.php + config.php
// под `php -S`, вместо Polza — поддельный сервер. Проверяет режим «как Node»
// (изоляция страницы, файлы на сервере сразу, сеть без CORS) и .vlhmn через PHP,
// а ключ ИИ остаётся на сервере.
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

// ── сайт без CORS (браузер напрямую его прочитать не может, Node — может) ──
const site = http.createServer((req, res) => { res.setHeader('Content-Type', 'text/plain; charset=utf-8'); res.end('секрет без CORS'); }).listen(0, '127.0.0.1');
await new Promise(r => site.on('listening', r));
const SITE = `http://127.0.0.1:${site.address().port}/data.txt`;

// ── «хостинг» ──
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vladx-shared-'));
fs.copyFileSync(path.join(ROOT, 'site/playground.html'), path.join(dir, 'playground.html'));
for (const f of ['index.php', 'vladx-common.php', 'vladx-llm.php', 'vladx-fs.php', 'vladx-fetch.php', 'vladpm-registry.php']) fs.copyFileSync(path.join(ROOT, 'deploy/shared', f), path.join(dir, f));
fs.writeFileSync(path.join(dir, 'config.php'), `<?php return ['polza_api_key' => '${KEY}', 'endpoint' => 'http://127.0.0.1:${polza.address().port}/v1/chat/completions',
  'model' => 'anthropic/claude-haiku-5.5', 'fetch_allow_private' => true];`);
const port = 18000 + Math.floor(Math.random() * 1000);
const php = spawn('php', ['-S', `127.0.0.1:${port}`, '-t', dir], { stdio: 'ignore', env: { ...process.env, http_proxy: '', HTTP_PROXY: '' } });
for (let i = 0; i < 50; i++) { try { await fetch(`http://127.0.0.1:${port}/vladx-llm.php`); break; } catch { await new Promise(r => setTimeout(r, 100)); } }
const php_ = (name, init) => fetch(`http://127.0.0.1:${port}/${name}`, init).then(r => r.json());

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
  await page.waitForFunction(() => /сервер/.test(document.getElementById('store-kind').textContent), null, { timeout: 10000 }).catch(() => {});
  check('1a', 'index.php изолирует страницу: режим «как Node», файлы на сервере',
    (await page.evaluate(() => self.crossOriginIsolated)) && (await page.innerText('#st-mode')).includes('как Node') && (await page.innerText('#store-kind')).includes('сервер'));

  // как Node: файл на сервере появляется сразу, пока программа ждёт ответа
  await page.fill('#code', 'file.запиши("заметки/день.txt", "записано по ходу")\nскажи "записал"\nспроси "продолжить?" и сохрани в ответ\nскажи file.прочитай("заметки/день.txt") + " / " + ответ');
  await page.click('#run');
  await page.waitForFunction(() => /ждёт ввода/.test(document.getElementById('status').textContent), null, { timeout: 30000 });
  const ws = await page.evaluate(() => localStorage.getItem('vladx.ws'));
  const onServer = await php_('vladx-fs.php', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-VladX-Workspace': ws }, body: JSON.stringify({ op: 'read', path: 'заметки/день.txt' }) });
  check('1b', 'файл записан на сервер сразу (программа ещё ждёт ответа)', onServer.ok && onServer.data === 'записано по ходу' && (await page.innerText('#output')).includes('записал'), JSON.stringify(onServer));
  await page.fill('#term-input', 'да'); await page.press('#term-input', 'Enter');
  await page.waitForTimeout(50); await idle();
  let o = await page.innerText('#output');
  check('1c', 'после ответа программа продолжила с того же места', o.includes('записано по ходу / да') && o.split('записал').length === 2, o);
  const other = await php_('vladx-fs.php', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-VladX-Workspace': 'f'.repeat(32) }, body: JSON.stringify({ op: 'list' }) });
  check('1d', 'у другого браузера своя рабочая папка', other.ok && other.files.length === 0, JSON.stringify(other));

  // как Node: сеть без CORS — через vladx-fetch.php
  o = await run(`скажи rete.получи("${SITE}")`);
  check('1e', 'rete.получи сайта без CORS — через сервер, как в Node', o.includes('секрет без CORS'), o);
  await page.click('#act-settings'); await page.uncheck('#netserver');
  o = await run(`скажи rete.получи("${SITE}")`);
  check('1f', 'напрямую из браузера тот же сайт недоступен (CORS) — понятная ошибка', o.includes('E-NET-001') && o.includes('через сервер'), o);
  await page.check('#netserver'); await page.click('#act-explorer');

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

  // ── vladpm: реестр на том же хостинге и панель «Пакеты» ──
  const { createHash } = await import('node:crypto');
  const files = { 'vladx.toml': '[проект]\nимя = "@тест/привет"\nверсия = "1.0.0"\nописание = "Приветствия"\nязык = "ru"\n', 'главная.vlstd': 'функция привет(кто) {\n  верни "Привет, " + кто\n}\n' };
  const h = createHash('sha256');
  for (const k of Object.keys(files).sort()) { h.update(k); h.update('\0'); h.update(files[k]); h.update('\0'); }
  const reg = (op, body, token) => php_('vladpm-registry.php?op=' + op, { method: 'POST', headers: { 'Content-Type': 'application/json', ...(token ? { Authorization: 'Bearer ' + token } : {}) }, body: JSON.stringify(body) });
  const acc = await reg('adduser', { user: 'тест', password: 'пароль-123' });
  const pub = await reg('publish', { bundle: { name: '@тест/привет', version: '1.0.0', files, integrity: 'sha256-' + h.digest('hex') }, meta: { description: 'Приветствия', lang: 'ru', dependencies: {} } }, acc.token);
  check('11a', 'реестр на хостинге: регистрация и публикация', acc.token && pub.ok, JSON.stringify([acc, pub]));
  await page.click('#act-packages');
  await page.waitForFunction(() => /пакетов: 1/.test(document.getElementById('pm-info').textContent), null, { timeout: 10000 }).catch(() => {});
  check('11b', 'панель «Пакеты» сама нашла реестр рядом', (await page.innerText('#pm-info')).includes('пакетов: 1'), await page.innerText('#pm-info'));
  await page.fill('#pm-q', 'привет');
  await page.waitForSelector('#pm-results button:has-text("добавить")', { timeout: 10000 });
  await page.click('#pm-results button:has-text("добавить")');
  await page.waitForFunction(() => /готово|ошибка/.test(document.getElementById('status').textContent) && /vladpm/.test(document.getElementById('status').textContent), null, { timeout: 30000 });
  o = await page.innerText('#output');
  const inst = await php_('vladx-fs.php', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-VladX-Workspace': ws }, body: JSON.stringify({ op: 'read', path: 'пакеты/@тест/привет/главная.vlstd' }) });
  check('11c', '«добавить» ставит пакет в рабочую папку на сервере (vladx.toml, vladx.lock, пакеты/)', o.includes('+ @тест/привет@1.0.0') && inst.ok, o + '\n' + JSON.stringify(inst));
  check('11d', 'зависимость видна в панели с версией', (await page.innerText('#pm-installed')).includes('@тест/привет') && (await page.innerText('#pm-installed')).includes('1.0.0'), await page.innerText('#pm-installed'));
  await page.selectOption('#fmt', 'vlstd');
  o = await run('из "@тест/привет" возьми привет\nскажи привет("мир")');
  check('11e', 'программа подключает пакет: из "@тест/привет" возьми …', o.includes('Привет, мир'), o);
  await page.click('#act-packages');
  await page.click('#pm-installed button');
  await page.waitForFunction(() => /remove.*(готово|ошибка)/.test(document.getElementById('status').textContent), null, { timeout: 30000 });
  const gone = await php_('vladx-fs.php', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-VladX-Workspace': ws }, body: JSON.stringify({ op: 'list' }) });
  check('11f', '× убирает пакет из папки и из vladx.toml', !gone.files.some(f => f.startsWith('пакеты/@тест')) && (await page.innerText('#output')).includes('- @тест/привет@1.0.0'), JSON.stringify(gone.files));
  check('11g', 'в консоли по-прежнему нет ошибок', errors.length === 0, errors.join('\n'));

  // Запасной режим: playground.html напрямую, без заголовков изоляции → «спроси» через повтор.
  const p2 = await (await browser.newContext()).newPage();
  await p2.goto(`http://127.0.0.1:${port}/playground.html`);
  await p2.waitForFunction(() => /готово|ошибка/.test(document.getElementById('status').textContent), null, { timeout: 30000 });
  const mode = await p2.innerText('#st-mode');
  await p2.fill('#code', 'скажи "раз"\nспроси "имя?" и сохрани в имя\nскажи "привет, {имя}"');
  await p2.click('#run');
  await p2.waitForFunction(() => /ждёт ввода/.test(document.getElementById('status').textContent), null, { timeout: 30000 });
  await p2.fill('#term-input', 'Ло'); await p2.press('#term-input', 'Enter');
  await p2.waitForFunction(() => /готово/.test(document.getElementById('status').textContent), null, { timeout: 30000 });
  const o2 = await p2.innerText('#output');
  check(10, 'без изоляции страницы — режим браузера, «спроси» продолжает программу повтором', mode.includes('браузер') && o2.includes('привет, Ло') && o2.split('раз').length === 2, mode + '\n' + o2);
} finally {
  await browser.close(); php.kill(); polza.close(); site.close();
  fs.rmSync(dir, { recursive: true, force: true });
}
console.log(`\n${passed} прошло, ${failed} упало`);
process.exit(failed ? 1 : 0);
