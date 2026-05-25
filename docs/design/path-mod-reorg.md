# Operations 按 path 重组 mod（已完成 — 历史设计文档）

把 `operations::` 下扁平的 `{Op}Request` / `{Op}Response` / `{Op}Server*`
按 **URL path + HTTP method** 组织进嵌套子模块。类型名统一收敛为
`Request` / `Response`（不再带 op 名前缀），用户通过 mod 路径导航。

**状态**：全部落地。`toac-build/src/path_mod.rs` 提供映射，
`operations::emit_operation` 在生成时 set_mod_path，`finish_operations`
按路径组装嵌套 `pub mod`。`QualifyComponents` rewrite 到
`crate::components::Foo`。相关 codegen test 见
`toac-build/tests/test_operations.rs`、`test_runtime_codegen.rs`、
`test_servers.rs`、`test_security.rs`——全部已迁移到新路径。本文件
保留作为实现参考。

## 动机

- **path 是唯一且总存在的**：不像 tag 可选且可多归属，path 天然唯一。
- **附属类型自然归属**：per-op server override、将来的 security 凭据等
  都归到同一个 mod 里，不需要 `GetPetServer` 这种复合名。
- **和 tag 分组不冲突**：将来要做 tag 分组，在 `ClientExt` 上加入口方法
  引用到这些 mod 里的类型即可，不用重构类型位置。

## 映射规则

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
```

## 关键决定

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

## 边界

- **根 `/`**：不嵌 mod，直接在 `operations::` 下放 `get::` / `post::` 等。
- **路径里空 segment (`//foo`)**：视为非法 spec，生成器报错。
- **segment 名与 Rust 关键字冲突**：`make_ident` 加 `r#` 处理。
- **path 有尾斜杠 `/pets/`**：忽略尾斜杠——`/pets` 和 `/pets/` 映射到同一
  mod。若同一 spec 真同时有两者当成冲突报错。
