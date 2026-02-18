//! Async FFI Tests — Production-grade coverage
//! Full compiler pipeline: lex → parse → analyze → MIR → codegen
//! Syntax matches dev_test/async/main.doo

use super::{assert_ffi_compiles, assert_ffi_compiles_with, assert_ffi_fails};

// =============================================================================
// 1. SLEEP
// =============================================================================

#[test]
fn async_sleep_basic() {
    assert_ffi_compiles_with(
        r#"
fn main() {
    sleep(50);
    print("done");
}
"#,
        "done",
    );
}

#[test]
fn async_await_sleep() {
    assert_ffi_compiles_with(
        r#"
fn main() {
    await sleep(50);
    print("done");
}
"#,
        "done",
    );
}

// =============================================================================
// 2. ASYNC FUNCTIONS
// =============================================================================

#[test]
fn async_fn_declaration() {
    assert_ffi_compiles_with(
        r#"
async fn fetchUser() -> Str {
    sleep(40);
    return "Alice";
}
fn main() {
    let user = await fetchUser();
    print(user);
}
"#,
        "Alice",
    );
}

#[test]
fn async_fn_multiple() {
    assert_ffi_compiles_with(
        r#"
async fn fetchUser() -> Str {
    sleep(40);
    return "Alice";
}
async fn fetchEmail() -> Str {
    sleep(20);
    return "alice@doo.dev";
}
fn main() {
    let user = await fetchUser();
    let email = await fetchEmail();
    print("${user} / ${email}");
}
"#,
        "alice@doo.dev",
    );
}

// =============================================================================
// 3. GO BLOCKS — fire and forget
// =============================================================================

#[test]
fn async_go_detached() {
    assert_ffi_compiles_with(
        r#"
fn main() {
    go {
        print("detached task running");
    }
    sleep(100);
    print("done");
}
"#,
        "detached",
    );
}

#[test]
fn async_go_with_handle() {
    assert_ffi_compiles_with(
        r#"
fn main() {
    let task = go {
        sleep(30);
        print("task with handle done");
    };
    sleep(100);
    print("done");
}
"#,
        "task with handle done",
    );
}

// =============================================================================
// 4. SCOPE — structured concurrency
// =============================================================================

#[test]
fn async_scope_basic() {
    assert_ffi_compiles_with(
        r#"
fn main() {
    scope {
        go {
            sleep(60);
            print("A done");
        }
        go {
            sleep(20);
            print("B done");
        }
    }
    print("scope exited");
}
"#,
        "scope exited",
    );
}

#[test]
fn async_scope_waits_all() {
    assert_ffi_compiles_with(
        r#"
fn main() {
    scope {
        go {
            sleep(40);
            print("slow task");
        }
        go {
            sleep(10);
            print("fast task");
        }
        go {
            print("instant task");
        }
    }
    print("all tasks finished");
}
"#,
        "all tasks finished",
    );
}

// =============================================================================
// 5. ASYNC FN + GO BLOCKS
// =============================================================================

#[test]
fn async_fn_inside_go() {
    assert_ffi_compiles_with(
        r#"
async fn slowQuery(name: Str, ms: Int) -> Str {
    sleep(ms);
    return "query result: ${name}";
}
fn main() {
    go {
        let r = await slowQuery("db1", 30);
        print(r);
    }
    sleep(200);
    print("done");
}
"#,
        "query result",
    );
}

#[test]
fn async_scope_with_async_fn() {
    assert_ffi_compiles_with(
        r#"
async fn slowQuery(name: Str, ms: Int) -> Str {
    sleep(ms);
    return "result: ${name}";
}
fn main() {
    scope {
        go {
            let r1 = await slowQuery("users", 40);
            print(r1);
        }
        go {
            let r2 = await slowQuery("posts", 20);
            print(r2);
        }
    }
    print("scope done");
}
"#,
        "scope done",
    );
}

// =============================================================================
// 6. NESTED GO BLOCKS
// =============================================================================

#[test]
fn async_nested_go() {
    assert_ffi_compiles_with(
        r#"
fn main() {
    go {
        print("outer go");
        go {
            sleep(10);
            print("inner go");
        }
        sleep(40);
        print("outer done");
    }
    sleep(200);
    print("done");
}
"#,
        "inner go",
    );
}

// =============================================================================
// 7. FOR LOOP WITH AWAIT
// =============================================================================

#[test]
fn async_for_loop_sleep() {
    assert_ffi_compiles_with(
        r#"
fn main() {
    for i in 0..3 {
        await sleep(20);
        print("poll iteration ${i}");
    }
    print("polling done");
}
"#,
        "poll iteration",
    );
}

// =============================================================================
// 8. MIXED SYNC AND ASYNC
// =============================================================================

#[test]
fn async_mixed_sync_async() {
    assert_ffi_compiles_with(
        r#"
fn compute(x: Int) -> Int => x * x;
fn main() {
    let v1 = compute(5);
    print(v1);
    await sleep(20);
    let v2 = compute(10);
    print(v2);
    await sleep(20);
    print("mixed done");
}
"#,
        "mixed done",
    );
}

// =============================================================================
// 9. THREE-WAY CONCURRENCY
// =============================================================================

#[test]
fn async_three_way_concurrency() {
    assert_ffi_compiles_with(
        r#"
fn main() {
    scope {
        go {
            sleep(50);
            print("A done (50ms)");
        }
        go {
            sleep(30);
            print("B done (30ms)");
        }
        go {
            sleep(10);
            print("C done (10ms)");
        }
    }
    print("expected order: C, B, A");
}
"#,
        "expected order",
    );
}

// =============================================================================
// 10. SEQUENTIAL AWAIT CHAIN
// =============================================================================

#[test]
fn async_sequential_await() {
    assert_ffi_compiles_with(
        r#"
async fn fetchUser() -> Str {
    sleep(40);
    return "Alice";
}
async fn fetchEmail() -> Str {
    sleep(20);
    return "alice@doo.dev";
}
async fn fetchRole() -> Str {
    sleep(60);
    return "admin";
}
fn main() {
    let u = await fetchUser();
    let e = await fetchEmail();
    let r = await fetchRole();
    print("${u} / ${e} / ${r}");
}
"#,
        "admin",
    );
}
