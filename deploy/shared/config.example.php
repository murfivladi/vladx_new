<?php
// Скопируйте в config.php (рядом с vladx-llm.php) или, лучше, в vladx-config.php
// на уровень выше public_html — туда веб-сервер не отдаёт файлы.
return [
    // Ключ: https://polza.ai/dashboard/api-keys
    'polza_api_key'    => 'ВСТАВЬТЕ_КЛЮЧ',

    // Модель для перевода .vlhmn. Дешёвая по умолчанию; точнее — anthropic/claude-sonnet-5.5.
    'model'            => 'anthropic/claude-haiku-5.5',
    'reasoning_effort' => 'low',

    // Другие сайты, которым можно пользоваться этим ИИ из браузера (свой сайт разрешён всегда).
    'allowed_origins'  => [],

    // Токен для CLI (латиница и цифры): vladx шлёт его из переменной VLADX_LLM_PROXY_TOKEN. Пусто — без токена.
    'access_token'     => '',

    // Лимиты, чтобы чужие не потратили баланс.
    'per_ip_per_hour'  => 60,
    'per_day'          => 1000,
];
