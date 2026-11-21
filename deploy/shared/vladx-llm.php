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
 * Совместим с PHP 7.2+, нужен curl (или allow_url_fopen). Общее — в vladx-common.php.
 */

require __DIR__ . '/vladx-common.php';

$config = vx_config();
vx_guard();

if (vx_method() === 'GET') {
    $ok = $config['polza_api_key'] !== '';
    vx_reply(200, [
        'available' => $ok,
        'model'     => $config['model'],
        'reason'    => $ok ? null : 'на сервере не задан ключ Polza (config.php)',
    ]);
}
if (vx_method() !== 'POST') {
    vx_reply(405, ['ok' => false, 'error' => 'только GET и POST']);
}
if ($config['polza_api_key'] === '') {
    vx_reply(503, ['ok' => false, 'error' => 'на сервере не задан ключ Polza (config.php)']);
}

// ───────────── проверка запроса ─────────────

$req = vx_json_body((int) $config['max_request_bytes']);
if (!isset($req['messages']) || !is_array($req['messages'])) {
    vx_reply(400, ['ok' => false, 'error' => 'ожидался JSON {"messages": [...]}']);
}
$messages = [];
foreach ($req['messages'] as $m) {
    if (!is_array($m) || !isset($m['role'], $m['content']) || !is_string($m['content'])
        || !in_array($m['role'], ['system', 'user', 'assistant'], true)) {
        vx_reply(400, ['ok' => false, 'error' => 'сообщение должно быть {"role": system|user|assistant, "content": строка}']);
    }
    $messages[] = ['role' => $m['role'], 'content' => $m['content']];
}
// Только диалог перевода VladX: системный промпт ядра + не больше одного повтора.
$n = count($messages);
if ($n < 2 || $n > 4 || $messages[0]['role'] !== 'system'
    || strpos($messages[0]['content'], 'в программу на языке VladX') === false) {
    vx_reply(400, ['ok' => false, 'error' => 'этот сервер переводит только .vlhmn для VladX']);
}

// ───────────── лимиты ─────────────

vx_limit('llm', (int) $config['per_ip_per_hour']);
if (!vx_bump('llm-day', (int) $config['per_day'], 86400)) {
    vx_reply(429, ['ok' => false, 'error' => 'дневной лимит запросов к ИИ на этом сервере исчерпан']);
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
        vx_reply(502, ['ok' => false, 'error' => 'Polza недоступна: ' . $err]);
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
        vx_reply(502, ['ok' => false, 'error' => 'Polza недоступна (нет curl и allow_url_fopen?)']);
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
    vx_reply(502, ['ok' => false, 'error' => $prefix . ' (' . $status . '): ' . $msg]);
}
$choice = isset($j['choices'][0]) ? $j['choices'][0] : null;
$finish = is_array($choice) && isset($choice['finish_reason']) ? (string) $choice['finish_reason'] : '';
if ($finish === 'length') {
    vx_reply(502, ['ok' => false, 'error' => 'ответ модели обрезан по длине']);
}
if ($finish === 'content_filter') {
    vx_reply(502, ['ok' => false, 'error' => 'модель отказалась отвечать (content_filter)']);
}
$content = is_array($choice) && isset($choice['message']['content']) ? $choice['message']['content'] : null;
if (!is_string($content)) {
    vx_reply(502, ['ok' => false, 'error' => 'в ответе Polza нет текста']);
}
vx_reply(200, ['ok' => true, 'content' => $content, 'model' => $config['model']]);
