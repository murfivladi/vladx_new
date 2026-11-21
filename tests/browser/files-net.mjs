// Браузерный сценарий: настоящий Chromium и vladx serve (страница изолирована — режим «как Node»):
// вывод по ходу, «спроси» ждёт ответа, файлы пишутся на диск сразу, сеть через сервер.
//   node tests/browser/files-net.mjs     (сценарий сам поднимает vladx serve с временной папкой файлов)
// Переменные: VLADX_URL (страница), VLADX_NET_URL + VLADX_NET_FIELD (JSON с CORS
// и числовым полем; по умолчанию api.github.com …/rust-lang/rust, stargazers_count),
// HTTPS_PROXY (прокси окружения для Chromium), VLADX_BROWSER_CA_SPKI (доверять CA прокси).
import { createRequire } from 'node:module';
import { spawn, execSync } from 'node:child_process';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
let chromium;
try { ({ chromium } = require('playwright')); } catch { ({ chromium } = require('playwright-core')); }

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const PORT = 18800 + Math.floor(Math.random() * 500);
const PAGE = `http://localhost:${PORT}/site/playground.html`;
const FILES = fs.mkdtempSync(path.join(os.tmpdir(), 'vladx-files-'));
const NET_URL = process.env.VLADX_NET_URL || 'https://api.github.com/repos/rust-lang/rust';
const NET_FIELD = process.env.VLADX_NET_FIELD || 'stargazers_count';

async function up() { try { return (await fetch(PAGE)).ok; } catch { return false; } }
const target = JSON.parse(execSync('cargo metadata --format-version 1 --no-deps', { cwd: ROOT })).target_directory;
execSync('cargo build -q -p vladx-cli', { cwd: ROOT, stdio: 'inherit' });
const server = spawn(path.join(target, 'debug', 'vladx'), ['serve', '--port', String(PORT), '--files', FILES], { cwd: ROOT, stdio: 'ignore' });
for (let i = 0; i < 80 && !(await up()); i++) await new Promise(r => setTimeout(r, 100));

const args = [];
if (process.env.VLADX_BROWSER_CA_SPKI) args.push('--ignore-certificate-errors-spki-list=' + process.env.VLADX_BROWSER_CA_SPKI);
const proxyEnv = process.env.HTTPS_PROXY || process.env.https_proxy;
const browser = await chromium.launch({
  args,
  ...(proxyEnv ? { proxy: { server: proxyEnv, bypass: 'localhost,127.0.0.1' } } : {}),
});
const page = await (await browser.newContext()).newPage();
const consoleErrors = [];
page.on('pageerror', e => consoleErrors.push(String(e)));
page.on('console', m => { if (m.type() === 'error') consoleErrors.push(m.text()); });

let passed = 0, failed = 0;
function check(n, name, ok, detail = '') {
  if (ok) { passed++; console.log(`  ok   ${n}. ${name}`); }
  else { failed++; console.log(`  FAIL ${n}. ${name}${detail ? '\n       ' + detail.replace(/\n/g, '\n       ') : ''}`); }
}
async function ready() {
  await page.waitForFunction(() => !document.getElementById('run').disabled, null, { timeout: 30000 });
  await page.waitForFunction(() => /готово|ошибка/.test(document.getElementById('status').textContent), null, { timeout: 30000 });
}
async function idle() {
  await page.waitForFunction(() => !/выполняется|переводит/.test(document.getElementById('status').textContent), null, { timeout: 60000 });
}
async function run(code) {
  await idle();
  await page.fill('#code', code);
  await page.click('#run');
  await page.waitForTimeout(50);
  await idle();
  // без строки-команды «$ vladx run …» в начале терминала
  return (await page.innerText('#output')).replace(/^\$ [^\n]*\n/, '');
}
async function filesPanel() { return page.$$eval('#files li:not(.empty) span:not(.ico)', els => els.map(e => e.textContent)); }

await page.goto(PAGE);
await ready();

check(0, 'страница изолирована — режим «как Node», файлы на диске', (await page.innerText('#st-mode')).includes('как Node') && (await page.innerText('#store-kind')).includes('диск'),
  await page.innerText('#st-mode') + ' / ' + await page.innerText('#store-kind'));

// 1. файлы без разрешения → E-FILE-001 и просьба человеку; после согласия программа идёт
await page.click('#act-settings');
await page.uncheck('#pfiles');
let out = await run('file.запиши("проба.txt", "1")\nскажи "записано"');
const askShown = await page.isVisible('#askpanel');
check(1, 'файлы без разрешения → E-FILE-001 + панель «Разрешить и запустить»', out.includes('E-FILE-001') && askShown, out);
await page.click('#allow');
await page.waitForFunction(() => /готово|ошибка/.test(document.getElementById('status').textContent) && !/выполняется/.test(document.getElementById('status').textContent));
out = await page.innerText('#output');
check('1b', 'после «Разрешить» программа запущена заново и прошла', out.includes('записано') && await page.isChecked('#pfiles'), out);
await run('file.удали("проба.txt")');

// 2. запись + чтение + список (включая вложенный путь)
out = await run(`file.запиши("заметка.txt", "первая строка")
пусть текст будет file.прочитай("заметка.txt")
скажи "текст: {текст}"
file.запиши("папка/вложенный.txt", "вложенные папки тоже работают")
скажи file.список()`);
check(2, 'запись + чтение + список', out.includes('текст: первая строка') && out.includes('папка/вложенный.txt'), out);
check('2b', 'файлы — настоящие файлы на диске сервера', fs.readFileSync(path.join(FILES, 'папка/вложенный.txt'), 'utf8') === 'вложенные папки тоже работают');

// 3. панель файлов показывает 2 файла
await page.waitForFunction(() => document.querySelectorAll('#files li:not(.empty)').length === 2, null, { timeout: 10000 }).catch(() => {});
let panel = await filesPanel();
check(3, 'панель файлов показывает 2 файла', panel.length === 2, JSON.stringify(panel));

// 4. после перезагрузки файлы на месте и читаются
await page.reload(); await ready(); await page.click('#act-settings');
out = await run('скажи file.прочитай("заметка.txt")\nскажи file.прочитай("папка/вложенный.txt")');
check(4, 'после перезагрузки файлы на месте', out.includes('первая строка') && out.includes('вложенные папки тоже работают'), out);

// 5. удаление → файла нет в списке
out = await run('file.удали("заметка.txt")\nскажи file.список()\nскажи file.есть("заметка.txt")');
check(5, 'удаление → файла нет в списке', !out.includes('заметка.txt') && out.includes('ложь'), out);

// 6. после перезагрузки удалённый файл не возвращается
await page.reload(); await ready(); await page.click('#act-settings');
out = await run('скажи file.список()');
panel = await filesPanel();
check(6, 'после перезагрузки удалённый не вернулся', !out.includes('заметка.txt') && out.includes('папка/вложенный.txt') && !panel.includes('заметка.txt'), out);

// 7. как Node: вывод виден сразу, «спроси» ждёт ответа, файл записан до конца программы
await idle();
await page.fill('#code', 'скажи "начало"\nfile.запиши("сразу.txt", "записано до вопроса")\nспроси "как тебя зовут?" и сохрани в имя\nспроси "город?" и сохрани в город\nскажи "здравствуй, {имя} из {город}"');
await page.click('#run');
await page.waitForFunction(() => /ждёт ввода/.test(document.getElementById('status').textContent), null, { timeout: 30000 });
const prompt1 = await page.innerText('#term-prompt');
const early = await page.innerText('#output');
const fileEarly = fs.existsSync(path.join(FILES, 'сразу.txt'));
check('7a', 'вывод по ходу: «начало» видно, пока программа ждёт ответа', early.includes('начало') && prompt1 === 'как тебя зовут?', early);
check('7b', 'файл записан сразу, до завершения программы', fileEarly);
await page.fill('#term-input', 'Мария'); await page.press('#term-input', 'Enter');
await page.waitForFunction(() => /город/.test(document.getElementById('term-prompt').textContent) && !document.getElementById('term-ask').hidden, null, { timeout: 30000 });
await page.fill('#term-input', 'Палермо'); await page.press('#term-input', 'Enter');
await page.waitForTimeout(50); await idle();
out = await page.innerText('#output');
check(7, 'спроси: два вопроса подряд, программа не перезапускалась', out.includes('как тебя зовут? Мария') && out.includes('здравствуй, Мария из Палермо') && out.split('начало').length === 2, out);

// 7c. модули: программа пишет модуль и тут же его импортирует (как require после writeFileSync)
out = await run('file.запиши("lib/мат.vlstd", "функция квадрат(x) \\{\\n  верни x * x\\n}\\nфункция сломай() \\{\\n  верни 1 / 0\\n}")\nиз "lib/мат" возьми квадрат, сломай\nскажи квадрат(9)\nсломай()');
const modErr = await page.$eval('#output', el => [...el.querySelectorAll('.diag')].map(d => d.textContent).join('\n'));
check('7c', 'импорт модуля из хранилища; ошибка в нём указывает на его файл и строку', out.includes('81') && modErr.includes('lib/мат.vlstd:стр 5') && modErr.includes('E-MATH-001'), out);
await page.click('#output .diag');
await page.waitForFunction(() => [...document.querySelectorAll('.tab.active')].some(t => t.textContent.includes('мат.vlstd')), null, { timeout: 10000 }).catch(() => {});
check('7d', 'клик по ошибке открывает модуль во вкладке', (await page.innerText('.tab.active')).includes('мат.vlstd'));
await page.click('.tab.active .close');

// 8. сеть напрямую из браузера (сайт с CORS) → число
await page.uncheck('#netserver');
out = await run(`пусть о будет rete.получиJson("${NET_URL}")\nпусть n будет о.${NET_FIELD}\nскажи n + 0 больше -1\nскажи n`);
check(8, `rete.получиJson напрямую (${new URL(NET_URL).host}) → число`, out.startsWith('истина'), out);

// 9. тот же запрос через сервер (как в Node, без CORS) → строка
await page.check('#netserver');
out = await run(`пусть т будет rete.получи("${NET_URL}")\nскажи testo.длина(т) больше 0\nскажи testo.кусок(т, 0, 1)`);
check(9, 'тот же запрос через сервер vladx-fetch.php → строка', out.startsWith('истина') && out.includes('{'), out);


// 10. перевод кода на другой язык прямо в браузере (ядро, без сервера)
await page.fill('#code', 'пусть x будет 2\nскажи x * 3');
await page.selectOption('#tolang', '1');
await page.waitForFunction(() => /переведено/.test(document.getElementById('status').textContent));
const itCode = await page.inputValue('#code');
check(10, 'перевод ru → it в браузере', itCode.includes('sia x = 2') && itCode.includes('dì x * 3') && (await page.inputValue('#lang')) === '1', itCode);
await page.selectOption('#lang', '0');
await idle();

// 11. .vlhmn: блоки кода разбираются без ИИ, перевод — через /hmn у vladx serve
await page.selectOption('#fmt', 'vlhmn');
out = await run('```\nскажи "из блока кода"\n```');
check(11, '.vlhmn из одного блока кода (через /hmn, без ИИ)', out.includes('из блока кода') && out.includes('ИИ понял так'), out);
await page.selectOption('#fmt', 'vlstd');

check(12, 'в консоли страницы нет ошибок', consoleErrors.length === 0, consoleErrors.join('\n'));

await browser.close();
server.kill();
fs.rmSync(FILES, { recursive: true, force: true });
console.log(`\n${passed} прошло, ${failed} упало`);
process.exit(failed ? 1 : 0);
