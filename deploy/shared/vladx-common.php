<?php
/**
 * VladX на shared-хостинге — общее для vladx-llm.php, vladx-fs.php, vladx-fetch.php:
 * настройки, JSON-ответы, проверка Origin, лимиты. Совместим с PHP 7.2+.
 */

declare(strict_types=1);

function vx_config(): array
{
    static $config = null;
    if ($config !== null) {
        return $config;
    }
    $config = [
        // ИИ (.vlhmn)
        'polza_api_key'        => '',
        'model'                => 'anthropic/claude-haiku-5.5',
        'reasoning_effort'     => 'low',
        'endpoint'             => 'https://polza.ai/api/v1/chat/completions',
        'max_tokens'           => 16000,
        'timeout'              => 120,
        'per_ip_per_hour'      => 60,
        'per_day'              => 1000,
        'max_request_bytes'    => 200000,
        // файлы программ на сервере (как fs в Node: запись сразу)
        'fs_enabled'           => true,
        'fs_quota_bytes'       => 10 * 1024 * 1024, // на одну рабочую папку
        'fs_max_files'         => 500,
        'fs_max_workspaces'    => 1000,
        'fs_per_ip_per_hour'   => 5000,
        // сеть программ через сервер (как в Node: без CORS)
        'fetch_enabled'        => true,
        'fetch_max_bytes'      => 5 * 1024 * 1024,
        'fetch_timeout'        => 20,
        'fetch_per_ip_per_hour' => 600,
        'fetch_allow_private'  => false, // true — разрешить адреса локальной сети (только для тестов)
        // общее
        'allowed_origins'      => [],
        'access_token'         => '',
        'data_dir'             => __DIR__ . '/vladx-data',
    ];
    foreach ([dirname(__DIR__) . '/vladx-config.php', __DIR__ . '/config.php'] as $file) {
        // Сначала конфиг уровнем выше (вне public_html), потом рядом со скриптом.
        if (is_file($file)) {
            $loaded = require $file;
            if (is_array($loaded)) {
                $config = array_merge($config, $loaded);
            }
            break;
        }
    }
    if ($config['polza_api_key'] === '' && getenv('POLZA_API_KEY')) {
        $config['polza_api_key'] = (string) getenv('POLZA_API_KEY');
    }
    return $config;
}

function vx_reply(int $status, array $body): void
{
    http_response_code($status);
    header('Content-Type: application/json; charset=utf-8');
    header('Cache-Control: no-store');
    echo json_encode($body, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES);
    exit;
}

function vx_client_ip(): string
{
    return isset($_SERVER['REMOTE_ADDR']) ? (string) $_SERVER['REMOTE_ADDR'] : 'unknown';
}

function vx_method(): string
{
    return isset($_SERVER['REQUEST_METHOD']) ? (string) $_SERVER['REQUEST_METHOD'] : 'GET';
}

/**
 * CORS и Origin. Свой сайт разрешён всегда; другие — из allowed_origins.
 * Чужой сайт в браузере получает отказ на любой запрос, кроме GET-проверки.
 * Без Origin (CLI, скрипты) — нужен access_token, если он задан.
 */
function vx_guard(bool $allowGetFromAnywhere = true): void
{
    $c = vx_config();
    $origin = isset($_SERVER['HTTP_ORIGIN']) ? (string) $_SERVER['HTTP_ORIGIN'] : '';
    $host = isset($_SERVER['HTTP_HOST']) ? (string) $_SERVER['HTTP_HOST'] : '';
    $same = $origin !== '' && in_array($origin, ['https://' . $host, 'http://' . $host], true);
    // Браузер не шлёт Origin на GET к своему же сайту — тогда видно Sec-Fetch-Site.
    $fetchSite = isset($_SERVER['HTTP_SEC_FETCH_SITE']) ? (string) $_SERVER['HTTP_SEC_FETCH_SITE'] : '';
    if ($origin === '' && $fetchSite === 'same-origin') {
        $origin = 'https://' . $host;
        $same = true;
    }
    $allowed = $origin !== '' && ($same || in_array('*', $c['allowed_origins'], true) || in_array($origin, $c['allowed_origins'], true));
    if ($allowed && !$same) {
        header('Access-Control-Allow-Origin: ' . $origin);
        header('Vary: Origin');
        header('Access-Control-Allow-Headers: Content-Type, X-VladX-Token, X-VladX-Workspace');
        header('Access-Control-Allow-Methods: GET, POST');
        header('Access-Control-Expose-Headers: X-VladX-Error');
    }
    // Площадка открыта с COEP: ответы этого сервера можно встраивать.
    header('Cross-Origin-Resource-Policy: same-origin');
    $m = vx_method();
    if ($m === 'OPTIONS') {
        http_response_code(204);
        exit;
    }
    if ($m === 'GET' && $allowGetFromAnywhere && !isset($_GET['url'])) {
        return;
    }
    if ($origin !== '' && !$allowed) {
        vx_reply(403, ['ok' => false, 'error' => 'запросы с сайта ' . $origin . ' не разрешены']);
    }
    if ($origin === '' && $c['access_token'] !== '') {
        $token = isset($_SERVER['HTTP_X_VLADX_TOKEN']) ? (string) $_SERVER['HTTP_X_VLADX_TOKEN'] : '';
        if (!hash_equals((string) $c['access_token'], $token)) {
            vx_reply(401, ['ok' => false, 'error' => 'нужен токен: задайте VLADX_LLM_PROXY_TOKEN']);
        }
    }
}

/** Счётчик в окне времени; false — лимит исчерпан. */
function vx_bump(string $name, int $limit, int $window): bool
{
    if ($limit <= 0) {
        return true;
    }
    $dir = vx_config()['data_dir'] . '/limits';
    if (!is_dir($dir) && !@mkdir($dir, 0700, true)) {
        return true; // нет места для счётчиков — лимит не применяется, но и не ломает работу
    }
    $fh = @fopen($dir . '/' . $name . '.json', 'c+');
    if ($fh === false) {
        return true;
    }
    flock($fh, LOCK_EX);
    $data = json_decode((string) stream_get_contents($fh), true);
    $now = time();
    if (!is_array($data) || $now - (int) ($data['start'] ?? 0) >= $window) {
        $data = ['start' => $now, 'count' => 0];
    }
    $ok = $data['count'] < $limit;
    if ($ok) {
        $data['count']++;
        ftruncate($fh, 0);
        rewind($fh);
        fwrite($fh, json_encode($data));
    }
    flock($fh, LOCK_UN);
    fclose($fh);
    return $ok;
}

function vx_limit(string $kind, int $perIpPerHour): void
{
    if (!vx_bump($kind . '-' . hash('sha256', vx_client_ip()), $perIpPerHour, 3600)) {
        vx_reply(429, ['ok' => false, 'error' => 'слишком много запросов с этого адреса, попробуйте позже']);
    }
}

/** Размер по-человечески: 100 байт, 12 КБ, 10 МБ. */
function vx_size(int $bytes): string
{
    if ($bytes < 1024) {
        return $bytes . ' байт';
    }
    if ($bytes < 1048576) {
        return round($bytes / 1024) . ' КБ';
    }
    return round($bytes / 1048576, 1) . ' МБ';
}

/** JSON-тело запроса не больше $max байт. */
function vx_json_body(int $max): array
{
    $raw = file_get_contents('php://input', false, null, 0, $max + 1);
    if ($raw === false || strlen($raw) > $max) {
        vx_reply(413, ['ok' => false, 'error' => 'запрос слишком большой']);
    }
    $j = json_decode($raw, true);
    if (!is_array($j)) {
        vx_reply(400, ['ok' => false, 'error' => 'ожидался JSON-объект']);
    }
    return $j;
}
