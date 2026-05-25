# Client 便捷层（ClientExt）— 设计文档（未落地）

**前置依赖**：[路径 mod 重组](path-mod-reorg.md) — 已完成。

目标：兑现 README 的"在 client 上提供一个叫 `{api_name}` 的方法"承诺。不改
`toac::ApiClient` 的类型参数，不破坏孤儿原则，让用户写
`client.get_pet(req).await` 而不是 `client.oneshot(req).await`。

**tag 分组暂不处理**——所有 op 扁平挂在一个 trait 上。等基础形态稳定、
有实际需求（spec 大到难以导航）时再引入。

## 架构

```rust
// 生成代码：pub mod client_ext { ... }

/// Blanket-impl'd extension trait。所有 operation 作为方法平铺。
/// 类型从 path mod 全路径引用。
pub trait ClientExt<S>: Sized {
    fn get_pet(&self, req: super::operations::pets::by_id::get::Request)
        -> impl ::std::future::Future<
            Output = Result<
                super::operations::pets::by_id::get::Response,
                ::toac::CallError<S::Error>,
            >,
        > + Send;

    fn create_pet(&self, req: super::operations::pets::post::Request)
        -> impl ::std::future::Future<
            Output = Result<
                super::operations::pets::post::Response,
                ::toac::CallError<S::Error>,
            >,
        > + Send;

    // ... 其余所有 op
}

impl<S, B> ClientExt<S> for ::toac::ApiClient<S>
where S: ::tower::Service<::toac::Request, Response = ::http::Response<B>>
         + Clone + Send + 'static,
      /* ...其余 tower bounds... */
{
    fn get_pet(&self, req: super::operations::pets::by_id::get::Request)
        -> /* impl Future ... */
    {
        use ::tower::ServiceExt;
        self.clone().oneshot(req)
    }
    // ...
}
```

## 关键决定

- **ExtTrait 路线**：孤儿原则强制——生成 crate 不能给 toac 的 `ApiClient` 加
  inherent method，只能走 trait。
- **所有 op 挂在同一个 `ClientExt` trait 上**：不分 tag、不分 namespace。形式
  最简单，用户 `use crate::client_ext::ClientExt;` 一次拿到所有方法。
- **方法签名吃 `Request` 值**：`fn get_pet(&self, req: GetPetRequest)`。不做
  positional 展开（参数多时不可读），不做自己写的 builder。
- **Builder 用 `bon`（opt-in）**：通过 `BuildOptions::use_bon = true` 触发，
  生成器给 Request struct 派生 `#[derive(::bon::Builder)]`。bon 自己处理
  必填字段的 typestate、`Option<T>` 自动识别为可选。用户在 Cargo.toml
  自己加 `bon = "..."` 依赖。和 `use_chrono` / `use_uuid` 风格一致。
- **返回类型 `impl Future + Send`**：和项目现有风格（`MakeRequest::make_request`、
  `ParseResponse::parse_response`）对齐。
- **方法名由 `operationId` 派生**：OAS 的 `operationId` 已经全局唯一，直接
  snake_case 转换即可；没有 `operationId` 的 fallback 到 `{method}_{path}`
  合成（现有 `operation_name` 逻辑已经这样做了）。

## 落地顺序

1. **生成 `client_ext` 模块**：
   - `ClientExt` trait 平铺所有 op 方法
   - Blanket impl for `::toac::ApiClient<S>`
   - 每个 op 的 impl body 就是 `self.clone().oneshot(req)` 一行
2. **Request builder**：`BuildOptions::use_bon` 开启时给每个 Request struct
   派生 `#[derive(::bon::Builder)]`。
3. **测试**：
   - `client.get_pet(req).await` 能编译能跑
   - 多个 op 的方法都在 trait 上
   - `use_bon = true` 时 `Request::builder().id("x").build()` 编译通过

## 远期

- **tag 分组**：spec 规模大到导航困难时再做。候选方案（`WithTag<'a, S, T>`
  + per-tag ZST marker + `ClientExt` 提供 namespace 入口方法）保留在 git
  历史中，届时可复用。
- **per-op `with_server` 同款入口**：Request 自己已有 `with_server` 方法，
  用户可在构造 request 时链式调用，不需要 trait 层额外支持。

## 后续问题

- 多 spec 并用时 `ClientExt` 重名：用户 `use a::client_ext::ClientExt as AExt; use b::client_ext::ClientExt as BExt;` 解决？还是生成器给 trait 加 spec 前缀？
- 方法名和 Rust 关键字冲突（`type`、`ref` 等）→ 用 `make_ident` 加 `r#`？
