<?php

declare(strict_types=1);

/**
 * PHP 单元测试 bootstrap
 *
 * 负责：
 *   1. 加载 Composer 自动加载器
 *   2. 如果 lychee_worker PHP 扩展未加载，加载 stub 函数签名
 *   3. 定义 STUB_DIR 常量（feature 测试用 ThinkPHP 应用根目录）
 */

// 1. Composer 自动加载
require_once __DIR__ . '/../vendor/autoload.php';

// 2. 扩展未加载时，加载 stub 函数，确保语法一致
if (!extension_loaded('lychee_worker')) {
    require_once __DIR__ . '/../stubs/lychee_worker.php';
}

// 3. feature 测试的 ThinkPHP 应用根目录
define('STUB_DIR', __DIR__ . '/stub');
