# Security / Auth 支持（已完成 — 历史设计文档）

目标：让生成的 client 能处理**绝大多数真实 API** 的鉴权需求——API Key 和 HTTP Auth (Basic/Bearer)——而无需用户自己写 tower layer 注入凭据。OAuth2 / OpenID Connect / mutualTLS 列为远期目标，当前阶段只做到"识别但不处理"。

**状态**：运行时（`toac/src/security.rs`）+ 生成器（`toac-build/src/security.rs`）+ codegen 测试（`toac-build/tests/test_security.rs`）全部落地；daytona 示例用生成的 `AuthConfig` 工作。本文件保留作为实现参考与远期目标记录。

## 本期范围（P0）

**支持的 scheme**：

- `type = "apiKey"`（`in: header` / `in: query` / `in: cookie`）
- `type = "http", scheme = "bearer"`
- `type = "http", scheme = "basic"`

这三类覆盖 95% 以上的公开 API（GitHub、Stripe、OpenAI、Slack、各家云厂商的 service key 等都在其中）。

**远期（P1/P2，本期不动）**：

- `type = "oauth2"` / `openIdConnect`：flow 种类多、要处理 token 刷新和外部 HTTP 交互，完整实现是一个独立项目的体量。
- `type = "mutualTLS"`：属于传输层的 TLS 配置，不是 HTTP header 注入，彻底不在 toac 范围内。
- `type = "http", scheme = <不是 bearer/basic>`（Digest、HOBA、Negotiate 等）：理论上走同一个 `Authorization` header 通道，但编码规则各不相同。本期不支持。

## 概念分层

鉴权要穿过三层，每层各有职责：

1. **Spec 侧**：`components.securitySchemes` 定义有哪些 scheme，根级 `security` 和 operation 级 `security` 声明每个 op 需要哪些 scheme。
2. **凭据侧（用户提供）**：每种 scheme 对应一个凭据类型（API key 字符串、Bearer token、用户名/密码）。本期凭据都是**同步、纯数据**——不涉及刷新、不涉及外部 HTTP。
3. **注入侧（运行时）**：凭据在请求发出前被写到 HTTP header / query / cookie，塞进 `Request`。

这三层必须解耦：spec 决定**形状**，用户提供**值**，runtime 负责**注入**。

## 核心抽象（运行时）

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

## 生成器产出

**per-scheme 凭据类型** (`pub mod security`)——本期三种形态：

```rust
// type: apiKey, in: header, name: X-API-Key
pub struct MyApiKeyCredential { pub value: String }

// type: http, scheme: bearer
pub struct MyBearerCredential { pub token: String }

// type: http, scheme: basic
pub struct MyBasicCredential { pub username: String, pub password: String }
```

注：每种 credential 都 `impl SecurityCredential`，`apply` 实际签名是 `-> impl Future + Send`。

**per-op SECURITY 声明**——每个 op 在 `impl Request` 暴露 spec 声明的 requirement：

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

## ApiClient 集成

最终选择：`ApiClient` 自己持 `Arc<dyn AuthSelector>`，不是独立 tower layer，也不加第二个泛型。

- `AuthSelector` trait 是 dyn-safe 的（返回 `AuthFuture<'a> = Pin<Box<dyn Future + Send>>`），让 `ApiClient` 保持单泛型
- 默认是 `NoAuth`，调 `.with_auth(auth)` 替换
- `NoAuth` 对 public op 放行、对有 security 要求的 op 返回 `CallError::Auth`——忘记 `with_auth` 会立刻报错而不是静默发不带凭据的请求
- per-op security 通过 `OperationSecurity` newtype 挂在 `http::Extensions` 上；生成器在 `MakeRequest` 里塞，`ApiClient::call` 路径里读
- `CallError::Auth(BoxError)` 和 `Encode` / `Transport` / `Decode` 平级

## 边界决定

- **遇到 OAuth2 / OIDC / mutualTLS / 未知 HTTP scheme**：在 `components` 里被静默跳过；op 的 `security` 引用它们时——**单个 alternative 全不支持** → 丢掉该 alternative（emit `cargo:warning=`），**全部 alternative 都挂** → 退化为空 SECURITY。
- **`security: []`**（显式空数组 = 无鉴权要求）：正确识别、layer 跳过注入。
- **根级 `security` + op 级 `security`**：op 级 override 根级（OAS 规范）。
- **spec 没声明 `security`**：视为 public，layer 跳过。
- **凭据缺失**：`CallError::Auth(BoxError)` 分支。

## 远期目标

- **OAuth2 flow 全支持**：生成一个 `OAuth2Credential`（也实现 `SecurityCredential`），内部持有用户注入的 token provider（如 `oauth2-rs` 或自定义），`apply` 时调 provider 拿 token + 刷新。不需要新 trait——本期保留的 async + Result 签名正好兜住。
- **OpenID Connect Discovery**：读 `openIdConnectUrl` 动态发现 endpoint；落点同上。
- **mutualTLS 集成文档**：写清楚如何在 hyper / reqwest 层配置客户端证书；toac 本身不出代码。
- **WithAuth op 级覆盖 wrapper**：需求出现时再做。
- **凭据轮换 / 多租户**：需求出现时再做。
