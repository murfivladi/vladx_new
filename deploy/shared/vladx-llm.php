<?php
/**
 * VladX — посредник LLM для .vlhmn на shared-хостинге.
 *
 * Ключ Polza и модель хранятся ТОЛЬКО здесь, на сервере (config.php).
 * Клиенты (площадка в браузере, CLI `vladx`) присылают диалог перевода,
 * скрипт добавляет ключ и модель и пересылает в Polza.ai.
 * Промпт, проверку ответа и повтор делает ядро VladX у клиента.
 *
 *   GET  vladx-llm.php  → {"available": true, "model": "..."}   (ключ не раскрывается)
 *   POST vladx-llm.php  {"messages": [{"role": "...", "content": "..."}]}
 *                       → {"ok": true, "content": "...", "model": "..."}
 *                       | {"ok": false, "error": "..."}
 *
 * Защита от чужого использования (это платный ключ):
 *   - модель и max_tokens задаёт сервер, клиент их выбрать не может;
 *   - принимается только диалог перевода VladX (проверка системного промпта);
 *   - лимиты: запросов в час с одного IP и запросов в сутки всего;
 *   - браузеры с чужих сайтов отсекаются по Origin; для CLI — необязательный токен.
 * Совместим с PHP 7.2+, нужен curl (или allow_url_fopen).
 */

declare(strict_types=1);

// ───────────── настройки ─────────────

$defaults = [
    'polza_api_key'        => '',
    'model'                => 'anthropic/claude-haiku-5.5',
    'reasoning_effort'     => 'low',          // '' — не передавать
    'endpoint'             => 'https://polza.ai/api/v1/chat/completions',
    'max_tokens'           => 16000,
    'timeout'              => 120,             // секунд на запрос к Polza
    'allowed_origins'      => [],              // [] — только тот же сайт; ['*'] — любые
    'access_token'         => '',              // если задан — нужен клиентам без разрешённого Origin (CLI)
    'per_ip_per_hour'      => 60,
    'per_day'              => 1000,
    'max_request_bytes'    => 200000,
    'data_dir'             => __DIR__ . '/vladx-data', // счётчики лимитов
];

$config = $defaults;
foreach ([dirname(__DIR__) . '/vladx-config.php', __DIR__ . '/config.php'] as $file) {
    // Сначала ищется конфиг уровнем выше (вне public_html), потом рядом со скриптом.
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

// ───────────── ответ ─────────────

function reply(int $status, array $body): void
{
    http_response_code($status);
    header('Content-Type: application/json; charset=utf-8');
    header('Cache-Control: no-store');
    echo json_encode($body, JSON_UNESCAPED_UNICODE);
    exit;
}

function client_ip(): string
{
    return isset($_SERVER['REMOTE_ADDR']) ? (string) $_SERVER['REMOTE_ADDR'] : 'unknown';
}

// ───────────── CORS и Origin ─────────────

$origin = isset($_SERVER['HTTP_ORIGIN']) ? (string) $_SERVER['HTTP_ORIGIN'] : '';
$host = isset($_SERVER['HTTP_HOST']) ? (string) $_SERVER['HTTP_HOST'] : '';
$sameSite = $origin !== '' && in_array($origin, ['https://' . $host, 'http://' . $host], true);
$allowedOrigin = $origin !== '' && ($sameSite
    || in_array('*', $config['allowed_origins'], true)
    || in_array($origin, $config['allowed_origins'], true));

if ($allowedOrigin && !$sameSite) {
    header('Access-Control-Allow-Origin: ' . $origin);
    header('Vary: Origin');
    header('Access-Control-Allow-Headers: Content-Type, X-VladX-Token');
    header('Access-Control-Allow-Methods: GET, POST');
}

$method = isset($_SERVER['REQUEST_METHOD']) ? $_SERVER['REQUEST_METHOD'] : 'GET';
if ($method === 'OPTIONS') {
    http_response_code(204);
    exit;
}

if ($method === 'GET') {
    $ok = $config['polza_api_key'] !== '';
    reply(200, [
        'available' => $ok,
        'model'     => $config['model'],
        'reason'    => $ok ? null : 'на сервере не задан ключ Polza (config.php)',
    ]);
}

if ($method !== 'POST') {
    reply(405, ['ok' => false, 'error' => 'только GET и POST']);
}

// Чужой сайт в браузере — отказ. Без Origin (CLI) — нужен токен, если он настроен.
if ($origin !== '' && !$allowedOrigin) {
    reply(403, ['ok' => false, 'error' => 'запросы с сайта ' . $origin . ' не разрешены']);
}
if ($origin === '' && $config['access_token'] !== '') {
    $token = isset($_SERVER['HTTP_X_VLADX_TOKEN']) ? (string) $_SERVER['HTTP_X_VLADX_TOKEN'] : '';
    if (!hash_equals((string) $config['access_token'], $token)) {
        reply(401, ['ok' => false, 'error' => 'нужен токен: задайте VLADX_LLM_PROXY_TOKEN']);
    }
}
if ($config['polza_api_key'] === '') {
    reply(503, ['ok' => false, 'error' => 'на сервере не задан ключ Polza (config.php)']);
}

// ───────────── проверка запроса ─────────────

$raw = file_get_contents('php://input', false, null, 0, (int) $config['max_request_bytes'] + 1);
if ($raw === false || strlen($raw) > (int) $config['max_request_bytes']) {
    reply(413, ['ok' => false, 'error' => 'запрос слишком большой']);
}
$req = json_decode($raw, true);
if (!is_array($req) || !isset($req['messages']) || !is_array($req['messages'])) {
    reply(400, ['ok' => false, 'error' => 'ожидался JSON {"messages": [...]}']);
}
$messages = [];
foreach ($req['messages'] as $m) {
    if (!is_array($m) || !isset($m['role'], $m['content']) || !is_string($m['content'])
        || !in_array($m['role'], ['system', 'user', 'assistant'], true)) {
        reply(400, ['ok' => false, 'error' => 'сообщение должно быть {"role": system|user|assistant, "content": строка}']);
    }
    $messages[] = ['role' => $m['role'], 'content' => $m['content']];
}
// Только диалог перевода VladX: системный промпт ядра + не больше одного повтора.
$n = count($messages);
if ($n < 2 || $n > 4 || $messages[0]['role'] !== 'system'
    || strpos($messages[0]['content'], 'в программу на языке VladX') === false) {
    reply(400, ['ok' => false, 'error' => 'этот сервер переводит только .vlhmn для VladX']);
}

// ───────────── лимиты ─────────────

function bump_counter(string $dir, string $name, int $limit, int $window): bool
{
    if ($limit <= 0) {
        return true;
    }
    if (!is_dir($dir) && !@mkdir($dir, 0700, true)) {
        return true; // нет места для счётчиков — лимит не применяется, но и не ломает работу
    }
    $file = $dir . '/' . $name . '.json';
    $fh = @fopen($file, 'c+');
    if ($fh === false) {
        return true;
    }
    flock($fh, LOCK_EX);
    $data = json_decode((string) stream_get_contents($fh), true);
    $now = time();
    if (!is_array($data) || $now - (int) ($data['start'] ?? 0) >= $window) {
        $data = ['start' => $now, 'count' => 0];
    }
    $allowed = $data['count'] < $limit;
    if ($allowed) {
        $data['count']++;
        ftruncate($fh, 0);
        rewind($fh);
        fwrite($fh, json_encode($data));
    }
    flock($fh, LOCK_UN);
    fclose($fh);
    return $allowed;
}

$dataDir = (string) $config['data_dir'];
if (!bump_counter($dataDir, 'ip-' . hash('sha256', client_ip()), (int) $config['per_ip_per_hour'], 3600)) {
    reply(429, ['ok' => false, 'error' => 'слишком много запросов с этого адреса, попробуйте через час']);
}
if (!bump_counter($dataDir, 'day', (int) $config['per_day'], 86400)) {
    reply(429, ['ok' => false, 'error' => 'дневной лимит запросов к ИИ на этом сервере исчерпан']);
}

// ───────────── запрос к Polza ─────────────

$body = [
    'model'      => $config['model'],
    'messages'   => $messages,
    'max_tokens' => (int) $config['max_tokens'],
];
if ($config['reasoning_effort'] !== '') {
    $body['reasoning_effort'] = $config['reasoning_effort'];
}
$payload = json_encode($body, JSON_UNESCAPED_UNICODE);
$headers = [
    'Content-Type: application/json',
    'Authorization: Bearer ' . $config['polza_api_key'],
    'User-Agent: vladx-llm-php/0.1',
];

$status = 0;
$respText = '';
if (function_exists('curl_init')) {
    $ch = curl_init((string) $config['endpoint']);
    curl_setopt_array($ch, [
        CURLOPT_POST           => true,
        CURLOPT_POSTFIELDS     => $payload,
        CURLOPT_HTTPHEADER     => $headers,
        CURLOPT_RETURNTRANSFER => true,
        CURLOPT_TIMEOUT        => (int) $config['timeout'],
        CURLOPT_CONNECTTIMEOUT => 15,
    ]);
    $out = curl_exec($ch);
    if ($out === false) {
        $err = curl_error($ch);
        curl_close($ch);
        reply(502, ['ok' => false, 'error' => 'Polza недоступна: ' . $err]);
    }
    $respText = (string) $out;
    $status = (int) curl_getinfo($ch, CURLINFO_RESPONSE_CODE);
    curl_close($ch);
} else {
    $ctx = stream_context_create(['http' => [
        'method'        => 'POST',
        'header'        => implode("\r\n", $headers),
        'content'       => $payload,
        'timeout'       => (int) $config['timeout'],
        'ignore_errors' => true,
    ]]);
    $out = @file_get_contents((string) $config['endpoint'], false, $ctx);
    if ($out === false) {
        reply(502, ['ok' => false, 'error' => 'Polza недоступна (нет curl и allow_url_fopen?)']);
    }
    $respText = $out;
    if (isset($http_response_header[0]) && preg_match('/\s(\d{3})\s/', $http_response_header[0], $mm)) {
        $status = (int) $mm[1];
    }
}

$j = json_decode($respText, true);
if ($status >= 400 || !is_array($j)) {
    $msg = is_array($j) && isset($j['error']['message']) ? (string) $j['error']['message'] : substr($respText, 0, 300);
    $why = [401 => 'ключ Polza не принят', 402 => 'недостаточно средств на балансе Polza', 429 => 'Polza: слишком много запросов'];
    $prefix = isset($why[$status]) ? $why[$status] : 'Polza ответила ' . $status;
    reply(502, ['ok' => false, 'error' => $prefix . ' (' . $status . '): ' . $msg]);
}
$choice = isset($j['choices'][0]) ? $j['choices'][0] : null;
$finish = is_array($choice) && isset($choice['finish_reason']) ? (string) $choice['finish_reason'] : '';
if ($finish === 'length') {
    reply(502, ['ok' => false, 'error' => 'ответ модели обрезан по длине']);
}
if ($finish === 'content_filter') {
    reply(502, ['ok' => false, 'error' => 'модель отказалась отвечать (content_filter)']);
}
$content = is_array($choice) && isset($choice['message']['content']) ? $choice['message']['content'] : null;
if (!is_string($content)) {
    reply(502, ['ok' => false, 'error' => 'в ответе Polza нет текста']);
}
reply(200, ['ok' => true, 'content' => $content, 'model' => $config['model']]);
