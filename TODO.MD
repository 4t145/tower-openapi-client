## 任务目标

这个仓库的任务是把OAS3库（https://docs.rs/oas3/latest/oas3/）中已经完成解析的结构体，变成Tower兼容的客户端结构。我们最终的目标是，能够让这个客户端完美访问对应的服务端。

### `oas3::Spec` 到 `TokenStream` 的转换
为了实现这个目标，首要任务就是编写一个转换器，能够完成`oas3::Spec` 到 `TokenStream` 的转换

一般来说，对于spec中的description字符串，我们要转换成rust中的`\\\`文档


### OAS 对象与 rust 项目的对应
首先生成的代码会提供一个`ApiClient<S>`类型，把用户可能传入的`HttpClient`(也就是泛型`S`)包装成一个可用的API客户端。

#### servers
servers 中不同的server应该作为 ApiClient Layer 的一个可以配置的选项。其中schema中提供的所有提供的server中的一项，应该作为一个类型提供，如果这个Server有变量，那么就是一个具有字段的结构体。
最后，所有的可选server作为一个Enum类型提供，作为ApiClient Layer 的一个可以配置的选项。
##### Server的命名规则
作为类型的名字，默认命名为`ServerOption{#index}`，如`ServerOption0`，`ServerOption1`。

#### path
每个 API 对应一个 Rust 类型 `Request`，落在按 URL path + HTTP method 嵌套的
子模块里。例如 `GET /pets/{id}` → `operations::pets::by_id::get::Request`，
其 `Response`、op-level server、future 的 security 凭据等附属类型同样归入
该 mod。

- 路径 literal segment → `to_snake_case`
- 路径参数 `{foo}` → `by_foo`
- HTTP method → `get` / `post` / `delete` / ...
- 根 `/` 不嵌，直接在 `operations` 下

每个 `Request` 会自动实现 `Operation`，因此可通过 `Service<Request>`
（即 `client.oneshot(req)`）调用；同时 `ClientExt` 生成一个按 `operationId`
命名的便捷方法（如 `client.get_pet(req).await`），两种调用方式并存。

#### tag
OAS 的 tags 目前**不**用于组织生成代码——类型的归宿由 path 决定（见上
条），tag 只是 spec 元数据。等基础稳定、规模大到需要分组导航时再引入
`ClientExt` 层面的 tag 入口。

#### webhook
out of scope（见"明确不做的范围"小节）。

#### components
每一个 Component 对应 Rust 中的一个类型。**已落地**：
`components.schemas` → `pub mod components { ... }` 里的
`struct` / `enum` / `type alias`，以及 `From` / `TryFrom` / `Display`
等相关 impl。其他 component kind（parameters / requestBodies /
responses / headers / examples / links / callbacks / pathItems）目前
通过 `oas3::Spec::resolve` 隐式展开，不生成独立 Rust 类型。

#### security
**已落地**。详见"Security / Auth 支持"小节（全部 7 步都已完成）。
生成 `pub mod security` 含 per-scheme credential + `AuthConfig` +
`AuthConfigBuilder` + `AuthSelector` impl；每个 op 在 `impl Request`
暴露 `pub const SECURITY` 并在 `make_request` 里注入
`http::Extensions::OperationSecurity`；`ApiClient::with_auth` 注入凭据。

#### externalDocs
**未实现**。README 里明确要"转化为文件顶层 `!//` 文档"，目前
`Info` / `license` / `contact` / `externalDocs` / 根 `description`
全部没有落到生成 crate 的元数据上。单独列在下面"未完成的大方向"。

# TODO

## 状态快照（2026-05-11）

### ✅ 已完成
- **运行时核心**：`MakeRequest` / `ParseResponse` / `Operation` / `ApiClient` 单泛型
- **Server**：`Server` trait + blanket impls（`&str` / `String` / `Arc<str>`）+ `WithServer` op-level override wrapper；生成器产出 `pub mod servers` 含 `ServerOption*` + `ApiServer` 聚合
- **Security 本期范围**：`SecurityCredential` / `AuthSelector` / `OperationSecurity` / `NoAuth` + 内置 API Key / Bearer / Basic 凭据；生成 `pub mod security` 含 `AuthConfig` + builder；`ApiClient::with_auth`
- **Codec 全套**：`BodyContentType` + `BodyEncoder<T>` + `BodyDecoder<O>` 三件套，覆盖 JSON / form / multipart / octet / text；生成器按 MIME 选 codec 并按 codec 决定 body Rust 类型（schema → struct / `Bytes` / `String` / `MultipartForm`）
- **CallError**：`Encode` / `Auth` / `Transport` / `Decode` 四分支
- **Operations 按 path 重组 mod**：`operations::<path>::<method>::{Request, Response, Server}`
- **Components**：`components.schemas` → Rust 类型（含递归自动 Box、allOf 扁平化、oneOf/anyOf → enum、discriminator → internally-tagged serde、string enum 带 Display）
- **Examples**：`examples/petstore`（hyper + HTTPS 真实调用）+ `examples/daytona`（生成 `AuthConfig` 驱动 bearer auth）

### ☐ 规划了但还没动
- **Client 便捷层 `ClientExt`**（见本文档下方"Client 便捷层"小节）：路径 mod 已经铺好，就差生成 `pub trait ClientExt<S>` + blanket impl
- **外部文档元数据**：`info` / `externalDocs` / `license` / `contact` → 生成 crate 的 `//!` doc（当前完全没处理）

### 🚫 明确不做
- **Webhooks / Callbacks**：服务端接收端点，不在客户端项目范围
- **Server codegen**：同上，只做客户端
- **OAuth2 flow 完整实现**：留给用户写自己的 `AuthSelector`（`SecurityCredential` 的 async + Result 签名已经为此留空间）
- **OpenID Connect Discovery**：同上
- **mutualTLS 证书管理**：属于传输层配置，不是 HTTP header 注入

### 🤔 还没决定
- **README 里承诺的 tag → mod 分组**：和 path mod 冲突，目前选择了 path，tag 仅作 spec 元数据；未来真需要分组导航时再加 `ClientExt` 入口方法
- **非 schemas 的 components**（parameters / responses / requestBodies / headers 等）：当前通过 `oas3::resolve` 隐式展开，未生成独立 Rust 类型。是否生成未决

---

## 实现功能缺失

- prefixItems

## Operations 按 path 重组 mod（✅ 已完成）

把 `operations::` 下扁平的 `{Op}Request` / `{Op}Response` / `{Op}Server*`
按 **URL path + HTTP method** 组织进嵌套子模块。类型名统一收敛为
`Request` / `Response`（不再带 op 名前缀），用户通过 mod 路径导航。

**状态**：全部落地。`toac-build/src/path_mod.rs` 提供映射，
`operations::emit_operation` 在生成时 set_mod_path，`finish_operations`
按路径组装嵌套 `pub mod`。`QualifyComponents` rewrite 到
`crate::components::Foo`。相关 codegen test 见
`toac-build/tests/test_operations.rs`、`test_runtime_codegen.rs`、
`test_servers.rs`、`test_security.rs`——全部已迁移到新路径。以下
保留原设计文档作为实现参考，不需要再做。

### 动机

- **path 是唯一且总存在的**：不像 tag 可选且可多归属，path 天然唯一。
- **附属类型自然归属**：per-op server override、将来的 security 凭据等
  都归到同一个 mod 里，不需要 `GetPetServer` 这种复合名。
- **和 tag 分组不冲突**：将来要做 tag 分组，在 `ClientExt` 上加入口方法
  引用到这些 mod 里的类型即可，不用重构类型位置。

### 映射规则

| spec 段 | mod 名 | 说明 |
|---|---|---|
| literal segment `foo` | `foo` | `to_snake_case` |
| literal segment `userProfiles` | `user_profiles` | 同上 |
| path param `{id}` | `by_id` | `by_` 前缀 + snake_case |
| path param `{petId}` | `by_pet_id` | 同上 |
| HTTP method | `get` / `post` / `delete` / ... | 小写 |
| 根 path `/` | （直接放 `operations` 根）| 不嵌 |

**示例**：

```
GET  /pets                           → operations::pets::get
POST /pets                           → operations::pets::post
GET  /pets/{id}                      → operations::pets::by_id::get
DELETE /users/{userId}/sessions      → operations::users::by_user_id::sessions::delete
```

每个最末 method mod 里住：
```
pub struct Request { ... }
pub enum Response { ... }
impl ::toac::MakeRequest for Request { ... }
impl ::toac::ParseResponse for Response { ... }
impl ::toac::Operation for Request { ... }

// 以及（如果该 op 声明了自己的 servers）:
pub struct ServerOption0;
pub enum Server { Option0(ServerOption0), Option1(ServerOption1) }
// 未来的 security 凭据同理
```

### 关键决定

- **方法嵌一层 mod**（而非类型名带 `Get` / `Post` 前缀）：`pets::by_id::get::Request`
  读起来比 `pets::by_id::GetRequest` 清晰，附属类型也不需要 `GetServer` 这种复合。
- **类型名固定 `Request` / `Response`**：用户通过 `use ...::pets::by_id::get;`
  导入 mod，然后 `get::Request`。也避免和 components 里的 schema 名字撞。
- **`ClientExt` 方法名仍由 `operationId` 派生**：path mod 是**类型归宿**，
  trait 方法名是**用户调用入口**，两者解耦。没有 `operationId` 的 op 仍走
  `{method}_{path}` fallback（现有 `operation_name` 逻辑）。
- **`QualifyComponents` 切换到绝对路径 `crate::components::Foo`**：operations
  模块层级从 1 级变成任意深度，visitor 按深度算 `super::` 不现实。这依赖
  "用户把 `toac::include_client!` 放在 crate 根"的默认假设——这本来就是
  `include_client!` 宏目前文档推荐的做法。
- **op-level server 的 `with_server` 方法**：仍在 `impl Request` 里；由于
  `Request` 现在位于 path mod 里，`with_server` 同理。无变化。

### 边界

- **根 `/`**：不嵌 mod，直接在 `operations::` 下放 `get::` / `post::` 等。
- **路径里空 segment (`//foo`)**：视为非法 spec，生成器报错。
- **segment 名与 Rust 关键字冲突**：`make_ident` 加 `r#` 处理（现有 naming
  已覆盖大部分场景）。
- **path 有尾斜杠 `/pets/`**：忽略尾斜杠——`/pets` 和 `/pets/` 映射到同一
  mod。若同一 spec 真同时有两者当成冲突报错。

### 落地顺序

1. **新增 `path_mod.rs`（或并入 `naming.rs`）**：把 path 字符串切分成
   segment 列表、每个 segment 映射成合法 Rust ident、产出嵌套 mod 路径。
2. **改造 `operations::emit_operation`**：
   - op 类型从 `{Op}Request`/`{Op}Response` 改成固定 `Request`/`Response`
   - 落地位置从 operations 根改成 path-derived 子 mod
   - per-op server override 类型同步移动
3. **`QualifyComponents` 改为绝对路径**：rewrite 目标从
   `super::components::Foo` 改成 `crate::components::Foo`。
4. **`finish_operations` 重写**：不能再简单 `#(#items)*` 平铺；要按 mod 路径
   构建嵌套 `pub mod` 树然后 emit。需要一个 helper 聚合每个 path-derived
   mod 的 items。
5. **测试迁移**：
   - compile-check / petstore / 所有现有 codegen 测试里的
     `use toac_compile_check::operations::{GetPetRequest, ...}` 改成
     `use toac_compile_check::operations::pets::by_id::get;`
   - codegen shape 测试的断言字符串更新
6. **新增专门测试**：
   - path literal → snake_case
   - path param → `by_*`
   - 同一 path 多 method 共处（`get::` 和 `post::` 并存）
   - op-level server 类型落在正确 mod 里
   - 组件引用通过 `crate::components::` 解析

### 做完后才能做的

- **Client 便捷层（下一节）**：方法体里的 `Request` 类型引用要写
  `operations::pets::by_id::get::Request`，依赖本节定下的 mod 结构。

---

## Client 便捷层（ClientExt）☐ 尚未落地

**前置依赖**：上一节"Operations 按 path 重组 mod"——**已完成**。现在可以开工。

目标：兑现 README 的"在 client 上提供一个叫 `{api_name}` 的方法"承诺。不改
`toac::ApiClient` 的类型参数，不破坏孤儿原则，让用户写
`client.get_pet(req).await` 而不是 `client.oneshot(req).await`。

**tag 分组暂不处理**——所有 op 扁平挂在一个 trait 上。等基础形态稳定、
有实际需求（spec 大到难以导航）时再引入。

### 架构

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

### 关键决定

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

### 落地顺序

1. **生成 `client_ext` 模块**：
   - `ClientExt` trait 平铺所有 op 方法
   - Blanket impl for `::toac::ApiClient<S>`
   - 每个 op 的 impl body 就是 `self.clone().oneshot(req)` 一行
2. **Request builder**：`BuildOptions::use_bon` 开启时给每个 Request struct
   派生 `#[derive(::bon::Builder)]`。
3. **测试**：
   - `client.get_pet(req).await` 能编译能跑
   - 多个 op 的方法都在 trait 上
   - `use_bon = true` 时 `GetPetRequest::builder().id("x").build()` 编译通过

### 远期

- **tag 分组**：spec 规模大到导航困难时再做。候选方案（`WithTag<'a, S, T>`
  + per-tag ZST marker + `ClientExt` 提供 namespace 入口方法）保留在 git
  历史中，届时可复用。
- **per-op `with_server` 同款入口**：Request 自己已有 `with_server` 方法，
  用户可在构造 request 时链式调用，不需要 trait 层额外支持。

### 后续问题

- 多 spec 并用时 `ClientExt` 重名：用户 `use a::client_ext::ClientExt as AExt; use b::client_ext::ClientExt as BExt;` 解决？还是生成器给 trait 加 spec 前缀？
- 方法名和 Rust 关键字冲突（`type`、`ref` 等）→ 用 `make_ident` 加 `r#`？

## Security / Auth 支持（✅ 本期范围全部完成）

目标：让生成的 client 能处理**绝大多数真实 API** 的鉴权需求——API Key 和 HTTP Auth (Basic/Bearer)——而无需用户自己写 tower layer 注入凭据。OAuth2 / OpenID Connect / mutualTLS 列为远期目标，当前阶段只做到"识别但不处理"。

**状态**：运行时（`toac/src/security.rs`）+ 生成器（`toac-build/src/security.rs`）+ codegen 测试（`toac-build/tests/test_security.rs`）全部落地；daytona 示例用生成的 `AuthConfig` 工作。以下内容保留作为实现参考与远期目标记录。

### 本期范围（P0）

**支持的 scheme**：
- `type = "apiKey"`（`in: header` / `in: query` / `in: cookie`）
- `type = "http", scheme = "bearer"`
- `type = "http", scheme = "basic"`

这三类覆盖 95% 以上的公开 API（GitHub、Stripe、OpenAI、Slack、各家云厂商的 service key 等都在其中）。

**远期（P1/P2，本期不动）**：
- `type = "oauth2"` / `openIdConnect`：flow 种类多、要处理 token 刷新和外部 HTTP 交互，完整实现是一个独立项目的体量。本期遇到这两种 scheme 在生成阶段**直接报 `Error::Unsupported`**，强制用户显式禁用它们或自己写 tower layer。
- `type = "mutualTLS"`：属于传输层的 TLS 配置，不是 HTTP header 注入，彻底不在 toac 范围内。遇到同样 `Error::Unsupported`。
- `type = "http", scheme = <不是 bearer/basic>`（Digest、HOBA、Negotiate 等）：理论上走同一个 `Authorization` header 通道，但编码规则各不相同。本期不支持。

### 概念分层

鉴权要穿过三层，每层各有职责：

1. **Spec 侧**：`components.securitySchemes` 定义有哪些 scheme，根级 `security` 和 operation 级 `security` 声明每个 op 需要哪些 scheme。
2. **凭据侧（用户提供）**：每种 scheme 对应一个凭据类型（API key 字符串、Bearer token、用户名/密码）。本期凭据都是**同步、纯数据**——不涉及刷新、不涉及外部 HTTP。
3. **注入侧（运行时）**：凭据在请求发出前被写到 HTTP header / query / cookie，塞进 `Request`。

这三层必须解耦：spec 决定**形状**，用户提供**值**，runtime 负责**注入**。

### 核心抽象（运行时）

```rust
// toac/src/security.rs（新）
pub trait SecurityCredential {
    /// 把凭据应用到请求上（加 header / 加 query / 加 cookie）。
    ///
    /// 本期三种 scheme 的实现都会返回 immediate-ready future 且不
    /// 报错；但签名保留 async + Result，这样将来 OAuth2 的
    /// token-refresh 版本接入时不是 breaking change。
    fn apply(
        &self,
        req: Request,
    ) -> impl Future<Output = Result<Request, BoxError>> + Send;
}
```

关键决定：

- **异步 + 可错**：`ApiClient::call` 本来就在 async 里，codec / transport / decode 每一环都已经可错——给 credential 加同款合约成本为 0，但避免了 OAuth2 接入时的 trait 改签名。
- **本期实现形态**：API key / Bearer / Basic 的实现都是 `async fn apply(...) { Ok(modified_req) }`，一个 `.await` 之后立即 ready，不产生实际暂停。
- **`apply` 吃 owned `Request` 吐 owned `Request`**：和 codec `encode_body`、`WithServer` 一致。
- **错误用 `BoxError`**：和 `CallError::Encode` / `CallError::Decode::Codec` 一致，codec-specific 错误类型擦除进 box。

### 生成器产出

**per-scheme 凭据类型** (`pub mod security`)——本期三种形态：

```rust
// type: apiKey, in: header, name: X-API-Key
pub struct MyApiKeyCredential { pub value: String }
impl SecurityCredential for MyApiKeyCredential {
    // 实际实现用 async fn；这里简写说明意图。
    async fn apply(&self, req: Request) -> Result<Request, BoxError> {
        /* 加 X-API-Key header */
        Ok(req)
    }
}

// type: http, scheme: bearer
pub struct MyBearerCredential { pub token: String }
impl SecurityCredential for MyBearerCredential {
    async fn apply(&self, req: Request) -> Result<Request, BoxError> {
        /* Authorization: Bearer <token> */
        Ok(req)
    }
}

// type: http, scheme: basic
pub struct MyBasicCredential { pub username: String, pub password: String }
impl SecurityCredential for MyBasicCredential {
    async fn apply(&self, req: Request) -> Result<Request, BoxError> {
        /* Authorization: Basic <b64> */
        Ok(req)
    }
}
```

注：`trait` 的真实签名是 `-> impl Future + Send`（见前一小节），这里三个 impl 用 `async fn` 语法糖写。

**per-op SECURITY 声明**——每个 op 在 `impl {Op}Request` 暴露 spec 声明的 requirement：

```rust
impl GetPetRequest {
    /// OR across entries, AND within an entry.
    /// 空切片表示 "public, no auth required"。
    pub const SECURITY: &'static [&'static [&'static str]] = &[&["MyApiKey"]];
}
```

本期用最朴素的形态 `[[scheme_name]]`——scope 槽位不生成（OAuth2 才用到）。

**聚合 `AuthConfig`**——类似 `ApiServer`，生成 `pub struct AuthConfig { my_api_key: Option<MyApiKeyCredential>, my_bearer: Option<MyBearerCredential>, ... }`，builder 填充：

```rust
let auth = AuthConfig::builder()
    .my_api_key("sk-...")
    .my_bearer("token-...")
    .build();
```

### ApiClient 集成（已实现）

最终选择：`ApiClient` 自己持 `Arc<dyn AuthSelector>`，不是独立 tower layer，也不加第二个泛型。

- `AuthSelector` trait 是 dyn-safe 的（返回 `AuthFuture<'a> = Pin<Box<dyn Future + Send>>`），让 `ApiClient` 保持单泛型
- 默认是 `NoAuth`，调 `.with_auth(auth)` 替换
- `NoAuth` 对 public op 放行、对有 security 要求的 op 返回 `CallError::Auth`——忘记 `with_auth` 会立刻报错而不是静默发不带凭据的请求
- per-op security 通过 `OperationSecurity` newtype 挂在 `http::Extensions` 上；生成器在 `MakeRequest` 里塞，`ApiClient::call` 路径里读
- `CallError::Auth(BoxError)` 和 `Encode` / `Transport` / `Decode` 平级

### 边界决定

- **遇到 OAuth2 / OIDC / mutualTLS**：生成阶段直接 `Error::Unsupported("OAuth2 security scheme not yet supported, see TODO.md")`，让 build.rs 崩出清晰错误。
- **遇到未知 HTTP auth scheme**（非 bearer/basic）：同上。
- **`security: []`**（显式空数组 = 无鉴权要求）：正确识别、layer 跳过注入。
- **根级 `security` + op 级 `security`**：op 级 override 根级（OAS 规范）。生成器合并规则按 spec 走。
- **spec 没声明 `security`**：视为 public，layer 跳过。
- **凭据缺失**：`CallError::Auth(BoxError)` 新分支，和 `Encode` / `Transport` / `Decode` 平级。

### 落地顺序

1. ✅ **运行时 trait**：`SecurityCredential`（async + Result）+ `AuthSelector`（dyn-safe）+ `OperationSecurity` + `NoAuth`，`CallError::Auth` 分支。
2. ✅ **内置凭据类型**：`ApiKeyCredential`（3 种 location）、`BearerCredential`、`BasicCredential`，附 8 个单元测试 + 2 个 `ApiClient` end-to-end 测试。
3. ✅ **`ApiClient::with_auth`**：`Arc<dyn AuthSelector>` 替换默认 `NoAuth`。
4. ✅ **scheme codegen**：`components.securitySchemes` → per-spec 凭据 newtype（delegate 到内置 `ApiKeyCredential`/`BearerCredential`/`BasicCredential`）+ 聚合 `AuthConfig`（实现 `AuthSelector`）+ builder。
5. ✅ **per-op `SECURITY` 常量 + Extensions 注入**：`impl Request { pub const SECURITY }` + `make_request` 末尾 `request.extensions_mut().insert(OperationSecurity(...))`。
6. ✅ **遇到不支持的 scheme → 降级处理**：OAuth2 / OIDC / mutualTLS / 非 bearer|basic 的 HTTP scheme 在 `components` 里被静默跳过；op 的 `security` 引用它们时——**单个 alternative 全不支持** → 丢掉该 alternative（emit `cargo:warning=`），**全部 alternative 都挂** → 退化为空 SECURITY。比原计划的 `Error::Unsupported` 更宽松，让 petstore 这种 oauth2-only 的 op 也能共存。
7. ✅ **codegen 测试**：`test_security.rs` 覆盖 bearer/apiKey/basic 各自形态 + 不支持 scheme 跳过 + 不支持 alternative 丢弃 + public op 空 SECURITY + 继承 spec-level。

### 远期目标（明确 P1/P2，本期不做）

- **OAuth2 flow 全支持**：生成一个 `OAuth2Credential`（也实现 `SecurityCredential`），内部持有用户注入的 token provider（如 `oauth2-rs` 或自定义），`apply` 时调 provider 拿 token + 刷新。不需要新 trait——本期保留的 async + Result 签名正好兜住。
- **OpenID Connect Discovery**：读 `openIdConnectUrl` 动态发现 endpoint；落点同上。
- **mutualTLS 集成文档**：写清楚如何在 hyper / reqwest 层配置客户端证书；toac 本身不出代码。
- **WithAuth op 级覆盖 wrapper**：需求出现时再做，不预先设计。
- **凭据轮换 / 多租户**：需求出现时再做。

### 后续问题（给未来的自己）

- `AuthConfig` 的 builder 方法名用 scheme 名的 snake_case 还是 fixed `.api_key()` / `.bearer()`？后者更规整但无法区分同类型多 scheme。倾向前者。

## Media Type / Codec 支持

目标：在 `toac` 中提供统一的 codec 模块（per-media-type），`toac-build`
根据 spec 声明挑选 codec，生成代码只负责填字段和调用 codec。

图例：☐ 计划支持 / ✅ 已支持 / — 不支持（规范允许但现实不存在，或该方向不适用）

### P0 — 阻塞基本可用性

| Media Type                            | Encode (req body) | Decode (resp body) | 模块                | 备注                                               |
| ------------------------------------- | ----------------- | ------------------ | ------------------- | -------------------------------------------------- |
| `application/json`                    | ✅                | ✅                 | `codec::json`       | `JsonEncoder` / `JsonDecoder`；生成器按 MIME 选 codec |
| `application/*+json`                  | ✅                | ✅                 | `codec::json`       | 复用 JSON 序列化，Content-Type 按 spec 覆写         |
| `application/x-www-form-urlencoded`   | ✅                | —                  | `codec::form`       | `FormEncoder` (serde_urlencoded)；payload 是任何 `Serialize` |
| `multipart/form-data`                 | ✅                | —                  | `codec::multipart`  | `MultipartForm` builder + `MultipartEncoder` (RFC 7578) |
| `application/octet-stream`            | ✅                | ✅                 | `codec::octet`      | `OctetEncoder` / `OctetDecoder`，payload 是 `bytes::Bytes`，覆盖 `image/*`、`audio/*`、`application/pdf` 等 |
| `text/plain`                          | ✅                | ✅                 | `codec::text`       | `TextEncoder` / `TextDecoder`，payload 是 `String`，UTF-8 校验 |
| *没有 body*                           | ✅                | ✅                 | —                   | 已支持                                             |

### P1 — 常见但可以稍后

| Media Type                                   | Encode | Decode | 模块              | 备注                                              |
| -------------------------------------------- | ------ | ------ | ----------------- | ------------------------------------------------- |
| `application/xml` / `text/xml`               | ☐      | ☐      | `codec::xml`      | feature-gated，`quick-xml` vs `serde-xml-rs` 待定 |
| `application/x-ndjson` / `application/jsonl` | —      | ☐      | `codec::ndjson`   | 流式 response；响应侧可能要 `Stream`，接口另设     |
| `text/event-stream` (SSE)                    | —      | ☐      | `codec::sse`      | 流式 response；AI 类 API 常用                     |

### P2 — 边缘情况

| Media Type                                | 备注                                                      |
| ----------------------------------------- | --------------------------------------------------------- |
| `application/yaml`                        | 配置类 API                                                |
| `application/cbor` / `application/msgpack`| 性能敏感 API                                              |
| `image/*` / `application/pdf` / `audio/*` | 本质是 octet-stream 别名，Content-Type 从 spec 原样搬过来 |
| `text/html`                               | 不列入路线                                                |

### P3 — 通配 / 兜底

- `*/*` → 视作 `octet-stream`（`Bytes`）
- 多个候选 content type → 按上述优先级打分选取，选择逻辑集中在 `toac-build::media`
- spec 声明了不支持的 MIME → 生成 `Error::Unsupported` 让 build.rs 报错，而非悄悄退化

### 跨媒体类型的共性工作

这些不属于"加一个 codec"，但与本计划强耦合：

- **`MediaType.encoding` 对象** — 只对 `multipart` / `urlencoded` 生效，定义每字段的
  `contentType` / `style` / `explode` / `allowReserved` / `headers`。multipart 必须读，
  urlencoded 读了更好。
- **参数 `style` / `explode`** — 属于 parameter 的事（path/query/header），与 body media
  type 无关，不走 codec 体系。独立跟踪。
- **响应 `headers`** — 与 body media type 无关，但重构 `ParseResponse` 时顺手能一起做。
- **`readOnly` / `writeOnly`** — schema 拆分问题，与 codec 无关。

### 落地顺序

1. codec 抽象骨架（只含 json）：把硬编码搬进 `toac::codec::json`，确认 generator 端
   的 codec 选择机制能工作。
2. `octet-stream` + `text/plain`：验证抽象。
3. `form-urlencoded`：引 `serde_urlencoded`。
4. `multipart/form-data`：encoding 对象 + 独立构造器接口 + per-part Content-Type。
5. P1 的 SSE / ndjson：流式接口与前四个不同，留到抽象稳定再做。

## 依赖功能缺失

以下 OpenAPI 3.1 / JSON Schema 2020-12 字段在 `oas3` 0.21 中未被解析或未被暴露，
按 "oas3 未支持的我们也不支持" 原则暂不实现。若上游补齐，可参考对应条目启用。

### `ObjectSchema` 字段

- **`$id`** — JSON Schema 2020-12 的 schema 身份 URI。`oas3::spec::ObjectSchema`
  没有对应字段；未知键会被 `spec_extensions` 过滤（只保留 `x-` 前缀）。
  - 影响：petstore3.1 的 `PetDetails { "$id": "/api/v31/components/schemas/petdetails" }`
    无法建立 `$id` → 组件索引，导致 `$ref: "/api/v31/..."` 无法解析。
  - 现状：退化为 `serde_json::Value` + 带原始 ref 路径的 doc comment。

- **`$anchor`** — JSON Schema 2020-12 的片段锚点。理由同 `$id`。
  - 影响：`$ref: "....#pet_details_id"` 这种锚点引用。
  - 现状：同上，退化为 `serde_json::Value`。

- **`$ref` 兄弟字段** — 在 `{"$ref": "...", "type": "integer", "description": "..."}`
  形式中，`type`/`description`/`format` 等兄弟关键字会被忽略。`oas3` 的
  `ObjectOrReference` 是 `#[serde(untagged)]` 枚举，匹配 `Ref` 分支后其他字段就丢了。
  - 影响：petstore3.1 的 `petDetailsId` 同时声明了 `type: integer, format: int64` 和 `$ref`。
  - 现状：只按 `$ref` 走。

- **`contains` / `maxContains` / `minContains`** — 数组包含约束。
- **`not`** — 模式否定。
- **`if` / `then` / `else` / `dependentRequired` / `dependentSchemas`** —
  条件子 schema 关键字。
- **`patternProperties` / `propertyNames` / `unevaluatedProperties` / `unevaluatedItems`** —
  对象 / 数组的附加约束。
- **`$defs`** — 本地子 schema 定义。
- **`$schema` / `$vocabulary` / `$dynamicRef` / `$dynamicAnchor`** — 词表与动态引用。
  - 影响：petstore3.1 的 `"$schema": "https://json-schema.org/draft/2020-12/schema"`
    与 `"$vocabulary": "..."` 被丢弃。目前我们也不使用这些。

### OpenAPI 3.0 向下兼容

- **`nullable: true`**（3.0 语法）— 3.1 用 `type: [T, 'null']` 表达，`oas3` 只认 3.1。
  - 影响：任何 `openapi: 3.0.x` 的 spec（如 e2b 样例）中声明 `nullable: true` 的字段
    会被当作不可为 null。
  - 现状：`is_nullable` 返回 `false`，生成的字段类型偏紧。

### 扩展字段

- **`x-enum-varnames`** — 事实标准，指定枚举 variant 的 Rust 名。`oas3` 把 `x-*`
  放进 `extensions: BTreeMap<String, serde_json::Value>`，我们能读到。
  - 状态：**oas3 已暴露**，未实现仅是我们这边的工作量。
  - 归类：非依赖缺失，按路线图推进即可。

### 其他

- **`Discriminator.mapping` 的外部 URI** — `Discriminator` 只有
  `property_name: String` 和 `mapping: Option<BTreeMap<String, String>>`；mapping
  value 是原始字符串，跨文档 URI 需要我们自己解析，但我们不会走到那一步。
