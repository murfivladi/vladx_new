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

    // Файлы программ на сервере (vladx-fs.php): у каждого браузера своя папка.
    'fs_enabled'        => true,
    'fs_quota_bytes'    => 10 * 1024 * 1024,  // на одну рабочую папку
    'fs_max_files'      => 500,
    'fs_max_workspaces' => 1000,

    // Сеть программ через сервер (vladx-fetch.php): как в Node, без CORS.
    'fetch_enabled'         => true,
    'fetch_max_bytes'       => 5 * 1024 * 1024,
    'fetch_per_ip_per_hour' => 600,

    // Реестр пакетов vladpm (vladpm-registry.php): регистрация открыта, пакет до 1 МБ.
    'registry_enabled'         => true,
    'registry_open_signup'     => true,
    'registry_max_package'     => 1024 * 1024,
    'registry_unpublish_hours' => 72,
];
