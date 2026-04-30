# TODO
## 实现功能缺失

- prefixItems

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
