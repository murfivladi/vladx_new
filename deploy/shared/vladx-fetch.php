<?php
/**
 * VladX — сеть программ через сервер, как в Node: без ограничений CORS браузера.
 *
 *   GET vladx-fetch.php           → {"available": true}
 *   GET vladx-fetch.php?url=…     → тело ответа сайта с его кодом и Content-Type;
 *                                   ошибка самого посредника — заголовок X-VladX-Error и текст.
 *
 * Защита: только http/https; адреса локальной и служебной сети запрещены
 * (сервер не станет ходить «внутрь» хостинга), каждый редирект проверяется
 * заново, IP фиксируется на время запроса; ограничены размер, время и частота.
 */

declare(strict_types=1);
require __DIR__ . '/vladx-common.php';

$c = vx_config();
vx_guard();

function fail(int $status, string $msg): void
{
    http_response_code($status);
    header('Content-Type: text/plain; charset=utf-8');
    header('X-VladX-Error: 1');
    header('Cache-Control: no-store');
    echo $msg;
    exit;
}

if (!isset($_GET['url'])) {
    vx_reply(200, ['available' => (bool) $c['fetch_enabled']]);
}
if (!$c['fetch_enabled']) {
    fail(503, 'сеть через этот сервер выключена');
}
if (!function_exists('curl_init')) {
    fail(503, 'на сервере нет расширения curl');
}
vx_limit('fetch', (int) $c['fetch_per_ip_per_hour']);

/** Проверить адрес и вернуть [host, port, ip] для фиксации DNS. */
function check_url(string $url, bool $allowPrivate): array
{
    $u = parse_url($url);
    if (!is_array($u) || !isset($u['scheme'], $u['host']) || !in_array(strtolower($u['scheme']), ['http', 'https'], true)) {
        fail(400, 'адрес должен начинаться с http:// или https://');
    }
    $host = trim($u['host'], '[]');
    $port = isset($u['port']) ? (int) $u['port'] : (strtolower($u['scheme']) === 'https' ? 443 : 80);
    $ips = filter_var($host, FILTER_VALIDATE_IP) ? [$host] : (gethostbynamel($host) ?: []);
    if (!$ips) {
        fail(502, 'не удалось найти сайт «' . $host . '»');
    }
    $ip = $ips[0];
    if (!$allowPrivate && !filter_var($ip, FILTER_VALIDATE_IP, FILTER_FLAG_NO_PRIV_RANGE | FILTER_FLAG_NO_RES_RANGE)) {
        fail(403, 'адрес «' . $host . '» ведёт во внутреннюю сеть — такие запросы запрещены');
    }
    return [$host, $port, $ip];
}

$url = (string) $_GET['url'];
$max = (int) $c['fetch_max_bytes'];
for ($hop = 0; $hop <= 5; $hop++) {
    [$host, $port, $ip] = check_url($url, (bool) $c['fetch_allow_private']);
    $body = '';
    $tooBig = false;
    $ch = curl_init($url);
    curl_setopt_array($ch, [
        CURLOPT_RESOLVE        => [$host . ':' . $port . ':' . $ip],
        CURLOPT_FOLLOWLOCATION => false,
        CURLOPT_TIMEOUT        => (int) $c['fetch_timeout'],
        CURLOPT_CONNECTTIMEOUT => 10,
        CURLOPT_USERAGENT      => 'vladx/0.1',
        CURLOPT_HEADER         => false,
        CURLOPT_PROTOCOLS      => CURLPROTO_HTTP | CURLPROTO_HTTPS,
        CURLOPT_WRITEFUNCTION  => function ($ch, $chunk) use (&$body, &$tooBig, $max) {
            if (strlen($body) + strlen($chunk) > $max) {
                $tooBig = true;
                return 0;
            }
            $body .= $chunk;
            return strlen($chunk);
        },
    ]);
    $ok = curl_exec($ch);
    $status = (int) curl_getinfo($ch, CURLINFO_RESPONSE_CODE);
    $type = (string) curl_getinfo($ch, CURLINFO_CONTENT_TYPE);
    $next = (string) curl_getinfo($ch, CURLINFO_REDIRECT_URL);
    $err = curl_error($ch);
    curl_close($ch);
    if ($tooBig) {
        fail(413, 'ответ сайта больше ' . vx_size($max));
    }
    if ($ok === false) {
        fail(502, 'запрос не удался: ' . $err);
    }
    if ($status >= 300 && $status < 400 && $next !== '') {
        $url = $next;
        continue;
    }
    http_response_code($status ?: 200);
    header('Content-Type: ' . ($type !== '' ? $type : 'application/octet-stream'));
    header('Cache-Control: no-store');
    echo $body;
    exit;
}
fail(508, 'слишком много перенаправлений');
