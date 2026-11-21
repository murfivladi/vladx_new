<?php
/**
 * Площадка VladX. Заголовки COOP/COEP включают изоляцию страницы: тогда
 * программа может по-настоящему ждать ответа на «спроси», как в Node
 * (SharedArrayBuffer + Atomics.wait в фоновом потоке).
 */
header('Cross-Origin-Opener-Policy: same-origin');
header('Cross-Origin-Embedder-Policy: require-corp');
header('Content-Type: text/html; charset=utf-8');
header('Cache-Control: no-cache');
readfile(__DIR__ . '/playground.html');
