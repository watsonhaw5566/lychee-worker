<?php

declare(strict_types=1);

namespace app\controller;

use think\facade\Cookie;
use think\Request;

class Index
{
    public function test(): string
    {
        Cookie::set('name', 'think');

        return 'test';
    }

    public function json(Request $request): mixed
    {
        return json($request->post());
    }
}
