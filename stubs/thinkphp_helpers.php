<?php

/*
 * PHPStan stub for ThinkPHP 8 global helper functions.
 *
 * These helpers are provided by the ThinkPHP framework at runtime and do not
 * exist as standalone files. This stub declares them for PHPStan so that
 * config files and route closures can call them without "function not found"
 * errors.
 *
 * NOTE: This file is loaded as a PHPStan stub (via phpstan.neon stubFiles).
 * It is NOT meant to be executed at runtime — the real implementations come
 * from the ThinkPHP framework. Return types are intentionally broad to avoid
 * depending on classes that may not be present during static analysis.
 */

/**
 * 读取环境变量。
 *
 * @param  string  $key  环境变量名
 * @param  mixed   $default  默认值
 * @return mixed
 */
function env(string $key, mixed $default = null): mixed
{
    return $default;
}

/**
 * 返回 JSON 响应。
 *
 * @param  mixed  $data
 * @param  int    $code
 * @param  array<string, string>  $header
 * @param  int    $options
 * @return mixed
 */
function json(mixed $data, int $code = 200, array $header = [], int $options = JSON_UNESCAPED_UNICODE): mixed
{
    return $data;
}

/**
 * 获取 public 目录绝对路径。
 *
 * @param  string  $path
 * @return string
 */
function public_path(string $path = ''): string
{
    return $path;
}

/**
 * 创建响应对象。
 *
 * @param  mixed   $content
 * @param  int     $code
 * @param  array<string, string>  $header
 * @return mixed
 */
function response(mixed $content = '', int $code = 200, array $header = []): mixed
{
    return $content;
}