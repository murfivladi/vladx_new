<?php
/**
 * VladX — файлы программ на сервере, как fs в Node: запись происходит сразу,
 * в момент вызова file.запиши, а не после завершения программы.
 *
 * У каждого браузера своя рабочая папка: заголовок X-VladX-Workspace
 * (случайный ключ из 32+ шестнадцатеричных знаков; на сервере — его хеш).
 *
 *   GET  vladx-fs.php → {"available": true, "kind": "server", "quota_bytes": …, "max_files": …}
 *   POST vladx-fs.php {"op": "list"}                         → {"ok": true, "files": [...]}
 *                     {"op": "read",   "path": "a/b.txt"}    → {"ok": true, "data": "..."}
 *                     {"op": "write",  "path": …, "data": …} → {"ok": true}
 *                     {"op": "delete", "path": …}            → {"ok": true}
 *                     {"op": "stat"}                          → {"ok": true, "used": байт, "files": N}
 *   Ошибка: {"ok": false, "error": "..."}.
 */

declare(strict_types=1);
require __DIR__ . '/vladx-common.php';

$c = vx_config();
vx_guard();

if (vx_method() === 'GET') {
    vx_reply(200, [
        'available'   => (bool) $c['fs_enabled'],
        'kind'        => 'server',
        'quota_bytes' => (int) $c['fs_quota_bytes'],
        'max_files'   => (int) $c['fs_max_files'],
    ]);
}
if (vx_method() !== 'POST') {
    vx_reply(405, ['ok' => false, 'error' => 'только GET и POST']);
}
if (!$c['fs_enabled']) {
    vx_reply(503, ['ok' => false, 'error' => 'файлы на этом сервере выключены']);
}
vx_limit('fs', (int) $c['fs_per_ip_per_hour']);

$ws = isset($_SERVER['HTTP_X_VLADX_WORKSPACE']) ? (string) $_SERVER['HTTP_X_VLADX_WORKSPACE'] : '';
if (!preg_match('/^[0-9a-f]{32,64}$/', $ws)) {
    vx_reply(400, ['ok' => false, 'error' => 'нужен ключ рабочей папки (X-VladX-Workspace)']);
}
$base = $c['data_dir'] . '/fs';
$root = $base . '/' . substr(hash('sha256', $ws), 0, 40);
$req = vx_json_body((int) $c['fs_quota_bytes'] + 65536);
$op = isset($req['op']) ? (string) $req['op'] : '';

function fs_path(string $p): string
{
    $p = str_replace('\\', '/', $p);
    $parts = [];
    foreach (explode('/', $p) as $seg) {
        if ($seg === '' || $seg === '.') {
            continue;
        }
        if ($seg === '..') {
            vx_reply(200, ['ok' => false, 'error' => 'путь «' . $p . '» выходит за пределы рабочей папки']);
        }
        if ($seg[0] === '.' || strlen($seg) > 120 || preg_match('/[\x00-\x1f<>:"|?*]/', $seg)) {
            vx_reply(200, ['ok' => false, 'error' => 'недопустимое имя «' . $seg . '»']);
        }
        $parts[] = $seg;
    }
    if (!$parts || strlen(implode('/', $parts)) > 400) {
        vx_reply(200, ['ok' => false, 'error' => 'пустое или слишком длинное имя файла']);
    }
    return implode('/', $parts);
}

function fs_list(string $root): array
{
    $out = [];
    if (!is_dir($root)) {
        return $out;
    }
    $it = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($root, FilesystemIterator::SKIP_DOTS));
    foreach ($it as $f) {
        if ($f->isFile()) {
            $out[] = ['path' => substr(str_replace('\\', '/', $f->getPathname()), strlen($root) + 1), 'size' => $f->getSize()];
        }
    }
    usort($out, function ($a, $b) { return strcmp($a['path'], $b['path']); });
    return $out;
}

switch ($op) {
    case 'list':
        vx_reply(200, ['ok' => true, 'files' => array_column(fs_list($root), 'path')]);
        // no break
    case 'stat':
        $all = fs_list($root);
        vx_reply(200, ['ok' => true, 'used' => array_sum(array_column($all, 'size')), 'files' => count($all)]);
        // no break
    case 'read':
        $rel = fs_path((string) ($req['path'] ?? ''));
        $file = $root . '/' . $rel;
        if (!is_file($file)) {
            vx_reply(200, ['ok' => false, 'error' => 'файл «' . $rel . '» не найден']);
        }
        vx_reply(200, ['ok' => true, 'data' => (string) file_get_contents($file)]);
        // no break
    case 'write':
        $rel = fs_path((string) ($req['path'] ?? ''));
        $data = isset($req['data']) && is_string($req['data']) ? $req['data'] : '';
        $file = $root . '/' . $rel;
        if (!is_dir($root)) {
            $count = is_dir($base) ? count(glob($base . '/*', GLOB_ONLYDIR) ?: []) : 0;
            if ($count >= (int) $c['fs_max_workspaces']) {
                vx_reply(200, ['ok' => false, 'error' => 'на сервере закончилось место для новых рабочих папок']);
            }
        }
        $all = fs_list($root);
        $used = 0;
        $exists = false;
        foreach ($all as $f) {
            if ($f['path'] === $rel) {
                $exists = true;
            } else {
                $used += $f['size'];
            }
        }
        if (!$exists && count($all) >= (int) $c['fs_max_files']) {
            vx_reply(200, ['ok' => false, 'error' => 'в рабочей папке слишком много файлов (предел ' . $c['fs_max_files'] . ')']);
        }
        if ($used + strlen($data) > (int) $c['fs_quota_bytes']) {
            vx_reply(200, ['ok' => false, 'error' => 'рабочая папка переполнена (предел ' . vx_size((int) $c['fs_quota_bytes']) . ')']);
        }
        if (is_dir($file)) {
            vx_reply(200, ['ok' => false, 'error' => '«' . $rel . '» — это папка']);
        }
        $dir = dirname($file);
        if (!is_dir($dir) && !@mkdir($dir, 0700, true)) {
            vx_reply(200, ['ok' => false, 'error' => 'папка для «' . $rel . '» не создана']);
        }
        if (@file_put_contents($file, $data, LOCK_EX) === false) {
            vx_reply(200, ['ok' => false, 'error' => 'файл «' . $rel . '» не записан']);
        }
        vx_reply(200, ['ok' => true]);
        // no break
    case 'delete':
        $rel = fs_path((string) ($req['path'] ?? ''));
        $file = $root . '/' . $rel;
        if (!is_file($file)) {
            vx_reply(200, ['ok' => false, 'error' => 'файл «' . $rel . '» не найден']);
        }
        if (!@unlink($file)) {
            vx_reply(200, ['ok' => false, 'error' => 'файл «' . $rel . '» не удалён']);
        }
        // пустые папки — убрать
        for ($d = dirname($file); strlen($d) > strlen($root) && @rmdir($d); $d = dirname($d)) {
        }
        vx_reply(200, ['ok' => true]);
        // no break
    default:
        vx_reply(400, ['ok' => false, 'error' => 'неизвестная операция «' . $op . '»']);
}
