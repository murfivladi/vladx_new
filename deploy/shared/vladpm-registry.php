<?php
/**
 * vladpm — реестр пакетов VladX на shared-хостинге (как registry.npmjs.org).
 * Хранение — файлы в data_dir/registry, без базы данных. PHP 7.2+.
 *
 * Чтение (открыто всем, как npm):
 *   GET ?op=info&name=…                 → карточка: версии, зависимости, README, владельцы
 *   GET ?op=fetch&name=…&version=…      → пакет: {"name","version","files":{путь:текст},"integrity"}
 *   GET ?op=search&q=…                  → {"results":[{"name","version","description","lang",…}]}
 * Учётные записи (POST JSON):
 *   ?op=adduser {"user","password"}      → {"user","token"}   регистрация открыта
 *   ?op=login   {"user","password"}      → {"user","token"}
 *   ?op=logout                           (токен — в заголовке Authorization: Bearer …)
 *   GET ?op=whoami                       → {"user"}
 * Публикация (нужен токен):
 *   ?op=publish   {"bundle":{…},"meta":{"dependencies","description","lang","readme",…}}
 *   ?op=unpublish {"name","version"|null}            только в течение 72 часов
 *   ?op=deprecate {"name","versions":[…],"message"}  "" — снять пометку
 *
 * Имена: буквы любого языка, цифры, «-», «_», «.»; область «@автор/имя» —
 * только для пользователя «автор». Пакет без области принадлежит первому, кто его опубликовал.
 */

declare(strict_types=1);
require __DIR__ . '/vladx-common.php';

$c = vx_config();
$cfg = array_merge([
    'registry_enabled'          => true,
    'registry_open_signup'      => true,
    'registry_max_package'      => 1024 * 1024,
    'registry_unpublish_hours'  => 72,
    'registry_signup_per_hour'  => 10,
    'registry_login_per_hour'   => 30,
    'registry_publish_per_hour' => 60,
    'registry_read_per_hour'    => 5000,
], $c);

// ───────────── CORS: читать может любой сайт, писать — свой сайт и CLI ─────────────
$origin = isset($_SERVER['HTTP_ORIGIN']) ? (string) $_SERVER['HTTP_ORIGIN'] : '';
$host = isset($_SERVER['HTTP_HOST']) ? (string) $_SERVER['HTTP_HOST'] : '';
$same = $origin === '' || in_array($origin, ['https://' . $host, 'http://' . $host], true)
    || in_array('*', $c['allowed_origins'], true) || in_array($origin, $c['allowed_origins'], true);
if ($origin !== '') {
    header('Access-Control-Allow-Origin: ' . $origin);
    header('Vary: Origin');
    header('Access-Control-Allow-Headers: Content-Type, Authorization, X-VladPM-Token');
    header('Access-Control-Allow-Methods: GET, POST');
}
header('Cross-Origin-Resource-Policy: cross-origin');
$method = vx_method();
if ($method === 'OPTIONS') {
    http_response_code(204);
    exit;
}
$op = isset($_GET['op']) ? (string) $_GET['op'] : '';
if (!$cfg['registry_enabled']) {
    vx_reply(503, ['ok' => false, 'error' => 'реестр на этом сервере выключен']);
}

// ───────────── хранилище ─────────────
$root = $c['data_dir'] . '/registry';
foreach (['', '/users', '/tokens', '/packages'] as $d) {
    if (!is_dir($root . $d) && !@mkdir($root . $d, 0700, true)) {
        vx_reply(500, ['ok' => false, 'error' => 'нет доступа к папке данных реестра']);
    }
}

function rg_h(string $s): string
{
    return hash('sha256', $s);
}

function rg_read(string $file): ?array
{
    if (!is_file($file)) {
        return null;
    }
    $j = json_decode((string) file_get_contents($file), true);
    return is_array($j) ? $j : null;
}

function rg_write(string $file, array $data): void
{
    $tmp = $file . '.' . bin2hex(random_bytes(4)) . '.tmp';
    $json = json_encode($data, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES);
    if ($json === false || file_put_contents($tmp, $json) === false || !rename($tmp, $file)) {
        @unlink($tmp);
        vx_reply(500, ['ok' => false, 'error' => 'не удалось сохранить данные реестра']);
    }
}

/** Взаимное исключение для изменения пакета или индекса. */
function rg_lock(string $file)
{
    $fh = fopen($file, 'c');
    if ($fh === false) {
        vx_reply(500, ['ok' => false, 'error' => 'не удалось заблокировать данные реестра']);
    }
    flock($fh, LOCK_EX);
    return $fh;
}

function rg_lower(string $s): string
{
    return function_exists('mb_strtolower') ? mb_strtolower($s, 'UTF-8') : strtolower($s);
}

function rg_sub(string $s, int $max): string
{
    return function_exists('mb_substr') ? mb_substr($s, 0, $max, 'UTF-8') : substr($s, 0, $max);
}

function rg_name_ok(string $n): bool
{
    return strlen($n) <= 200
        && preg_match('/^(@[\p{L}\p{N}][\p{L}\p{N}._-]{0,59}\/)?[\p{L}\p{N}][\p{L}\p{N}._-]{0,59}$/u', $n) === 1
        && rg_lower($n) === $n;
}

function rg_version_ok(string $v): bool
{
    return preg_match('/^\d{1,9}\.\d{1,9}\.\d{1,9}(-[\p{L}\p{N}.-]{1,40})?$/u', $v) === 1;
}

function rg_user_ok(string $u): bool
{
    return preg_match('/^[\p{L}\p{N}][\p{L}\p{N}_-]{1,39}$/u', $u) === 1;
}

function rg_need_name(string $n): void
{
    if (!rg_name_ok($n)) {
        vx_reply(400, ['ok' => false, 'error' => 'имя пакета «' . $n . '»: строчные буквы, цифры, «-», «_», «.»; область — @автор/имя']);
    }
}

function rg_pkg_dir(string $name): string
{
    global $root;
    return $root . '/packages/' . rg_h($name);
}

function rg_token(): string
{
    foreach (['HTTP_AUTHORIZATION', 'REDIRECT_HTTP_AUTHORIZATION'] as $k) {
        if (isset($_SERVER[$k]) && stripos((string) $_SERVER[$k], 'Bearer ') === 0) {
            return trim(substr((string) $_SERVER[$k], 7));
        }
    }
    // Часть хостингов не передаёт Authorization в PHP — запасной заголовок.
    return isset($_SERVER['HTTP_X_VLADPM_TOKEN']) ? trim((string) $_SERVER['HTTP_X_VLADPM_TOKEN']) : '';
}

/** Кто вошёл (или отказ). */
function rg_user(): string
{
    global $root;
    $t = rg_token();
    if ($t === '' || preg_match('/^[a-f0-9]{64}$/', $t) !== 1) {
        vx_reply(401, ['ok' => false, 'error' => 'нужен вход: vladpm login']);
    }
    $rec = rg_read($root . '/tokens/' . rg_h($t) . '.json');
    if ($rec === null) {
        vx_reply(401, ['ok' => false, 'error' => 'токен недействителен — войдите снова: vladpm login']);
    }
    return (string) $rec['user'];
}

function rg_new_token(string $user): string
{
    global $root;
    $t = bin2hex(random_bytes(32));
    rg_write($root . '/tokens/' . rg_h($t) . '.json', ['user' => $user, 'created' => time()]);
    return $t;
}

function rg_integrity(array $files): string
{
    ksort($files, SORT_STRING);
    $ctx = hash_init('sha256');
    foreach ($files as $p => $content) {
        hash_update($ctx, (string) $p . "\0" . (string) $content . "\0");
    }
    return 'sha256-' . hash_final($ctx);
}

function rg_path_ok(string $p): bool
{
    if ($p === '' || $p[0] === '/' || strpos($p, '\\') !== false || strpos($p, ':') !== false) {
        return false;
    }
    foreach (explode('/', $p) as $seg) {
        if ($seg === '' || $seg === '.' || $seg === '..' || $seg[0] === '.') {
            return false;
        }
    }
    return true;
}

function rg_cmp(string $a, string $b): int
{
    $pa = explode('-', $a, 2);
    $pb = explode('-', $b, 2);
    $c = version_compare($pa[0], $pb[0]);
    if ($c !== 0) {
        return $c;
    }
    if (!isset($pa[1]) || !isset($pb[1])) {
        return isset($pa[1]) ? -1 : (isset($pb[1]) ? 1 : 0);
    }
    return strnatcmp($pa[1], $pb[1]);
}

function rg_latest(array $versions): string
{
    $all = array_keys($versions);
    $stable = array_values(array_filter($all, function ($v) {
        return strpos((string) $v, '-') === false;
    }));
    $pick = $stable ? $stable : $all;
    usort($pick, 'rg_cmp');
    return $pick ? (string) end($pick) : '';
}

/** Индекс для поиска: имя → краткие сведения. */
function rg_index_update(string $name, ?array $meta): void
{
    global $root;
    $lock = rg_lock($root . '/index.lock');
    $idx = rg_read($root . '/index.json');
    if ($idx === null) {
        $idx = [];
    }
    if ($meta === null || !$meta['versions']) {
        unset($idx[$name]);
    } else {
        $latest = rg_latest($meta['versions']);
        $v = $meta['versions'][$latest];
        $idx[$name] = [
            'name'        => $name,
            'version'     => $latest,
            'description' => (string) ($v['description'] ?? ''),
            'lang'        => (string) ($v['lang'] ?? ''),
            'deprecated'  => (string) ($v['deprecated'] ?? ''),
            'owners'      => $meta['owners'],
            'updated'     => (int) ($v['published'] ?? time()),
        ];
    }
    rg_write($root . '/index.json', $idx);
    flock($lock, LOCK_UN);
    fclose($lock);
}

function rg_meta_public(array $m): array
{
    $latest = rg_latest($m['versions']);
    foreach ($m['versions'] as $v => $meta) {
        $m['versions'][$v]['dependencies'] = (object) ($meta['dependencies'] ?? []);
    }
    return [
        'name'        => $m['name'],
        'description' => $latest !== '' ? ($m['versions'][$latest]['description'] ?? null) : null,
        'latest'      => $latest,
        'owners'      => $m['owners'],
        'versions'    => (object) $m['versions'],
        'readme'      => (string) ($m['readme'] ?? ''),
        'created'     => (int) ($m['created'] ?? 0),
    ];
}

// ───────────── чтение ─────────────
if ($method === 'GET') {
    vx_limit('registry-read', (int) $cfg['registry_read_per_hour']);
    if ($op === '') {
        $idx = rg_read($root . '/index.json');
        vx_reply(200, ['registry' => 'vladpm', 'api' => 1, 'packages' => $idx ? count($idx) : 0, 'signup' => (bool) $cfg['registry_open_signup']]);
    }
    if ($op === 'info') {
        $name = (string) ($_GET['name'] ?? '');
        rg_need_name($name);
        $m = rg_read(rg_pkg_dir($name) . '/meta.json');
        if ($m === null || !$m['versions']) {
            vx_reply(404, ['ok' => false, 'error' => 'пакет «' . $name . '» не найден в реестре']);
        }
        vx_reply(200, rg_meta_public($m));
    }
    if ($op === 'fetch') {
        $name = (string) ($_GET['name'] ?? '');
        $ver = (string) ($_GET['version'] ?? '');
        rg_need_name($name);
        if (!rg_version_ok($ver)) {
            vx_reply(400, ['ok' => false, 'error' => 'версия «' . $ver . '» — вида 1.2.3']);
        }
        $file = rg_pkg_dir($name) . '/' . rg_h($ver) . '.json';
        if (!is_file($file)) {
            vx_reply(404, ['ok' => false, 'error' => $name . '@' . $ver . ' не найден в реестре']);
        }
        header('Content-Type: application/json; charset=utf-8');
        header('Cache-Control: public, max-age=31536000, immutable');
        readfile($file);
        exit;
    }
    if ($op === 'search') {
        $q = trim(rg_lower((string) ($_GET['q'] ?? '')));
        $idx = rg_read($root . '/index.json');
        $res = [];
        foreach ($idx ?: [] as $p) {
            $hay = rg_lower($p['name'] . ' ' . $p['description'] . ' ' . implode(' ', $p['owners']));
            if ($q === '' || strpos($hay, $q) !== false) {
                $p['score'] = $q !== '' && strpos(rg_lower($p['name']), $q) !== false ? 1 : 0;
                $res[] = $p;
            }
        }
        usort($res, function ($a, $b) {
            return [$b['score'], $b['updated']] <=> [$a['score'], $a['updated']];
        });
        vx_reply(200, ['results' => array_slice($res, 0, 50), 'total' => count($res)]);
    }
    if ($op === 'whoami') {
        vx_reply(200, ['user' => rg_user()]);
    }
    vx_reply(400, ['ok' => false, 'error' => 'неизвестная операция «' . $op . '»']);
}

if ($method !== 'POST') {
    vx_reply(405, ['ok' => false, 'error' => 'только GET и POST']);
}
if (!$same) {
    vx_reply(403, ['ok' => false, 'error' => 'изменять реестр с сайта ' . $origin . ' нельзя']);
}
$body = vx_json_body((int) $cfg['registry_max_package'] * 2 + 65536);

// ───────────── учётные записи ─────────────
if ($op === 'adduser' || $op === 'login') {
    $user = rg_lower(trim((string) ($body['user'] ?? '')));
    $pass = (string) ($body['password'] ?? '');
    if (!rg_user_ok($user)) {
        vx_reply(400, ['ok' => false, 'error' => 'имя пользователя: 2–40 букв, цифр, «-» или «_»']);
    }
    $file = $root . '/users/' . rg_h($user) . '.json';
    if ($op === 'adduser') {
        vx_limit('registry-signup', (int) $cfg['registry_signup_per_hour']);
        if (!$cfg['registry_open_signup']) {
            vx_reply(403, ['ok' => false, 'error' => 'регистрация на этом реестре закрыта']);
        }
        if (strlen($pass) < 8) {
            vx_reply(400, ['ok' => false, 'error' => 'пароль — не короче 8 знаков']);
        }
        $lock = rg_lock($root . '/users.lock');
        if (is_file($file)) {
            vx_reply(409, ['ok' => false, 'error' => 'имя «' . $user . '» уже занято — войдите: vladpm login']);
        }
        rg_write($file, ['user' => $user, 'hash' => password_hash($pass, PASSWORD_DEFAULT), 'created' => time()]);
        flock($lock, LOCK_UN);
        vx_reply(200, ['ok' => true, 'user' => $user, 'token' => rg_new_token($user)]);
    }
    vx_limit('registry-login', (int) $cfg['registry_login_per_hour']);
    $u = rg_read($file);
    if ($u === null || !password_verify($pass, (string) $u['hash'])) {
        vx_reply(401, ['ok' => false, 'error' => 'неверное имя или пароль']);
    }
    vx_reply(200, ['ok' => true, 'user' => $user, 'token' => rg_new_token($user)]);
}

if ($op === 'logout') {
    $t = rg_token();
    if ($t !== '' && preg_match('/^[a-f0-9]{64}$/', $t) === 1) {
        @unlink($root . '/tokens/' . rg_h($t) . '.json');
    }
    vx_reply(200, ['ok' => true]);
}

$me = rg_user();

/** Пакет, которым можно управлять: владелец или своя область. */
function rg_owned(string $name, string $me): array
{
    $m = rg_read(rg_pkg_dir($name) . '/meta.json');
    if ($m === null) {
        vx_reply(404, ['ok' => false, 'error' => 'пакет «' . $name . '» не найден в реестре']);
    }
    if (!in_array($me, $m['owners'], true)) {
        vx_reply(403, ['ok' => false, 'error' => 'вы не владелец пакета «' . $name . '» (владельцы: ' . implode(', ', $m['owners']) . ')']);
    }
    return $m;
}

if ($op === 'publish') {
    vx_limit('registry-publish', (int) $cfg['registry_publish_per_hour']);
    $b = $body['bundle'] ?? null;
    $meta = is_array($body['meta'] ?? null) ? $body['meta'] : [];
    if (!is_array($b) || !is_array($b['files'] ?? null)) {
        vx_reply(400, ['ok' => false, 'error' => 'нет пакета']);
    }
    $name = (string) ($b['name'] ?? '');
    $ver = (string) ($b['version'] ?? '');
    rg_need_name($name);
    if (!rg_version_ok($ver)) {
        vx_reply(400, ['ok' => false, 'error' => 'версия «' . $ver . '» — вида 1.2.3']);
    }
    if ($name[0] === '@') {
        $scope = substr($name, 1, strpos($name, '/') - 1);
        if ($scope !== $me) {
            vx_reply(403, ['ok' => false, 'error' => 'область @' . $scope . ' принадлежит пользователю «' . $scope . '», а вы — «' . $me . '»; публикуйте как @' . $me . '/…']);
        }
    }
    $size = 0;
    foreach ($b['files'] as $p => $content) {
        if (!is_string($content) || !rg_path_ok((string) $p)) {
            vx_reply(400, ['ok' => false, 'error' => 'путь «' . $p . '» в пакете недопустим']);
        }
        $size += strlen((string) $p) + strlen($content);
    }
    if ($size > (int) $cfg['registry_max_package']) {
        vx_reply(413, ['ok' => false, 'error' => 'пакет больше ' . vx_size((int) $cfg['registry_max_package'])]);
    }
    if (!isset($b['files']['vladx.toml'])) {
        vx_reply(400, ['ok' => false, 'error' => 'в пакете нет vladx.toml']);
    }
    $integrity = rg_integrity($b['files']);
    if (!hash_equals($integrity, (string) ($b['integrity'] ?? ''))) {
        vx_reply(400, ['ok' => false, 'error' => 'контрольная сумма пакета не совпала — пакет повреждён при передаче']);
    }
    $deps = [];
    foreach (is_array($meta['dependencies'] ?? null) ? $meta['dependencies'] : [] as $dn => $range) {
        if (!rg_name_ok((string) $dn) || !is_string($range) || strlen($range) > 100) {
            vx_reply(400, ['ok' => false, 'error' => 'зависимость «' . $dn . '» записана неверно']);
        }
        $deps[(string) $dn] = $range;
    }
    $dir = rg_pkg_dir($name);
    if (!is_dir($dir) && !@mkdir($dir, 0700, true)) {
        vx_reply(500, ['ok' => false, 'error' => 'нет доступа к папке пакета']);
    }
    $lock = rg_lock($dir . '/lock');
    $m = rg_read($dir . '/meta.json');
    if ($m === null) {
        $m = ['name' => $name, 'owners' => [$me], 'versions' => [], 'gone' => [], 'created' => time()];
    } elseif (!in_array($me, $m['owners'], true)) {
        vx_reply(403, ['ok' => false, 'error' => 'имя «' . $name . '» уже занято пользователем ' . implode(', ', $m['owners']) . ' — выберите другое или @' . $me . '/' . $name]);
    }
    if (isset($m['versions'][$ver]) || in_array($ver, $m['gone'], true)) {
        vx_reply(409, ['ok' => false, 'error' => $name . '@' . $ver . ' уже публиковался — поднимите версию в vladx.toml']);
    }
    $s = function ($k, $max) use ($meta) {
        return is_string($meta[$k] ?? null) ? rg_sub($meta[$k], $max) : null;
    };
    $lang = in_array($meta['lang'] ?? '', ['ru', 'it', 'scn'], true) ? $meta['lang'] : 'ru';
    rg_write($dir . '/' . rg_h($ver) . '.json', ['name' => $name, 'version' => $ver, 'files' => (object) $b['files'], 'integrity' => $integrity]);
    $m['versions'][$ver] = array_filter([
        'dependencies' => (object) $deps,
        'integrity'    => $integrity,
        'description'  => $s('description', 300),
        'lang'         => $lang,
        'license'      => $s('license', 60),
        'author'       => $s('author', 120),
        'published'    => time(),
        'publisher'    => $me,
        'size'         => $size,
    ], function ($v) {
        return $v !== null;
    });
    uksort($m['versions'], 'rg_cmp');
    if (rg_latest($m['versions']) === $ver) {
        $m['readme'] = $s('readme', 200000) ?? '';
    }
    rg_write($dir . '/meta.json', $m);
    flock($lock, LOCK_UN);
    rg_index_update($name, $m);
    $page = (isset($_SERVER['HTTPS']) && $_SERVER['HTTPS'] !== 'off' ? 'https://' : 'http://') . $host
        . rtrim(str_replace('\\', '/', dirname((string) ($_SERVER['SCRIPT_NAME'] ?? '/'))), '/') . '/vladpm.html#/' . $name;
    vx_reply(200, ['ok' => true, 'name' => $name, 'version' => $ver, 'integrity' => $integrity, 'page' => $page]);
}

if ($op === 'unpublish') {
    $name = (string) ($body['name'] ?? '');
    rg_need_name($name);
    $dir = rg_pkg_dir($name);
    $lock = rg_lock($dir . '/lock');
    $m = rg_owned($name, $me);
    $ver = isset($body['version']) && $body['version'] !== null ? (string) $body['version'] : null;
    $targets = $ver === null ? array_keys($m['versions']) : [$ver];
    if ($ver !== null && !isset($m['versions'][$ver])) {
        vx_reply(404, ['ok' => false, 'error' => $name . '@' . $ver . ' не найден']);
    }
    $limit = (int) $cfg['registry_unpublish_hours'] * 3600;
    foreach ($targets as $v) {
        if (time() - (int) $m['versions'][$v]['published'] > $limit) {
            vx_reply(403, ['ok' => false, 'error' => $name . '@' . $v . ' опубликован больше ' . $cfg['registry_unpublish_hours'] . ' ч назад — снять нельзя, пометьте устаревшим: vladpm deprecate']);
        }
    }
    foreach ($targets as $v) {
        @unlink($dir . '/' . rg_h((string) $v) . '.json');
        unset($m['versions'][$v]);
        $m['gone'][] = (string) $v;
    }
    rg_write($dir . '/meta.json', $m);
    flock($lock, LOCK_UN);
    rg_index_update($name, $m);
    vx_reply(200, ['ok' => true, 'removed' => array_map('strval', $targets)]);
}

if ($op === 'deprecate') {
    $name = (string) ($body['name'] ?? '');
    rg_need_name($name);
    $msg = rg_sub((string) ($body['message'] ?? ''), 300);
    $dir = rg_pkg_dir($name);
    $lock = rg_lock($dir . '/lock');
    $m = rg_owned($name, $me);
    $done = [];
    foreach (is_array($body['versions'] ?? null) ? $body['versions'] : [] as $v) {
        $v = (string) $v;
        if (isset($m['versions'][$v])) {
            if ($msg === '') {
                unset($m['versions'][$v]['deprecated']);
            } else {
                $m['versions'][$v]['deprecated'] = $msg;
            }
            $done[] = $v;
        }
    }
    if (!$done) {
        vx_reply(404, ['ok' => false, 'error' => 'таких версий у «' . $name . '» нет']);
    }
    rg_write($dir . '/meta.json', $m);
    flock($lock, LOCK_UN);
    rg_index_update($name, $m);
    vx_reply(200, ['ok' => true, 'versions' => $done]);
}

vx_reply(400, ['ok' => false, 'error' => 'неизвестная операция «' . $op . '»']);
