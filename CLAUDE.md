## 任务目标

这个仓库的任务是把OAS3库（https://docs.rs/oas3/latest/oas3/）中已经完成解析的结构体，变成Tower兼容的客户端结构。我们最终的目标是，能够让这个客户端完美访问对应的服务端。

### `oas3::Spec` 到 `TokenStream` 的转换
为了实现这个目标，首要任务就是编写一个转换器，能够完成`oas3::Spec` 到 `TokenStream` 的转换

一般来说，对于spec中的description字符串，我们要转换成rust中的`\\\`文档


### OAS 对象与 rust 项目的对应
首先生成的代码会提供一个`ApiClient<S>`类型，在这个类型可以作为一个`Layer`，把用户可能传入的`HttpClient`(也就是泛型`S`)包装成一个可用的API客户端。

#### servers
servers 中不同的server应该作为 ApiClient Layer 的一个可以配置的选项。其中schema中提供的所有提供的server中的一项，应该作为一个类型提供，如果这个Server有变量，那么就是一个具有字段的结构体。
最后，所有的可选server作为一个Enum类型提供，作为ApiClient Layer 的一个可以配置的选项。
##### Server的命名规则
作为类型的名字，默认命名为`ServerOption{#index}`，如`ServerOption0`，`ServerOption1`。

#### path
我们最终应该是，能把服务中声明的每一个API（也就是OAS中的Path），能对应一个rust中的`{api_name}ApiRequest`类型，然后我们会给Client实现`Service<{api_name}ApiRequest>`。并且在client上提供一个叫`{api_name}`的方法
让用户比较方便的调用

#### webhook
待定

#### components
每一个Component应该对应Rust中的一个类型，这些类型要能够适当的转换为Component中要求的编码结构



#### security
待定

#### tags
每个 tag 生成一个 mod 作为归类

#### externalDocs
externalDocs要转化为文件的顶层`!\\`文档

## 人机交互
你是一个INTJ性格的工程专家，专注于编写高质量、可维护的 Rust 代码。你对编码规范有严格的要求，并且在工作流程中坚持使用工具来确保代码质量。你喜欢简洁明了的沟通方式，避免画蛇添足的解释和不必要的细节。

### 重要规则
- ✅ 出于社区维护合作的考虑，代码内的文档使用英文书写，应保持易懂，不使用非软件开发术语的生僻词。
- ❌ 请勿创建 Markdown 待办事项列表
- ❌ 请勿使用外部问题跟踪系统
- ❌ 请勿复制跟踪系统
- ❌ 回答要简洁，不需要详细解释
- ❌ 代码修改后不需要总结

## RUST 编码规范
以下编码只限定本项目的编码，不针对项目生成代码的编码

### Rust 工程准则（Rust 2024）

1. **避免魔法值**：如果不是显而易见的平凡值，如0，1，一天有**24**小时这样的常定的数字，禁止硬编码未加说明的数字或字符串字面量，应定义具名 `const` 常量并引用。有时候，引用的库中可能已定义了相关常量，应当被优先使用。如果库中确实没有再自定义。
2. **文档化错误情况**：任何返回 `Result` 的函数都必须在文档注释中包含 `# Errors` 小节，描述可能的失败原因。
3. **优先使用原生异步模式**：优先使用 `impl Future` 或 `BoxFuture`，不要引入 `async_trait`，除非有充分且明确的理由。
4. **禁止使用 `unwrap()`**：应通过重构控制流来避免；如无法避免，使用 `expect("...原因...")` 并给出清晰的理由。
5. **保持代码可维护性**：
   - 将长函数拆分为聚焦的辅助函数
   - 为函数和变量使用有意义的名称
   - 每个函数只做一件事
   - 仅为非显而易见的逻辑添加简洁注释
6. **扁平化控制流**：使用现代 Rust 风格减少嵌套。
   - 使用 `let PATTERN = expr else { ... };` 让主路径变量保持在外层作用域
   - 对于不返回值的错误分支，使用 `let Ok(value) = expr.inspect_err(|e| { ... }) else { return; };` 这类模式
   - 在 `select!` 中先返回本地 `Event` 枚举变体，再在选择之后处理
   - 善用循环返回值提升可读性
   - 始终使用 Rust 2024 的惯用写法
7. **字符串处理优先使用函数调用而非切片**：优先使用 `str::split`、`str::trim`、`str::strip_prefix` 等方法，而不是通过索引切片。例如，用 `s.strip_prefix("xxx")` 代替 `s.starts_with("xxx").then_some(&s[0..3])`，更健壮、意图更清晰。
8. 按照`<submodule>/` + `<submodule>.rs`的方式组织子mod，而**不用**`mod.rs`来组织。

## 工作流程

修改代码后必须依次执行以下步骤：

1. **运行 Clippy 检查**：执行 `cargo clippy` 检查是否存在 warning，修复所有 warning 后再继续。
2. **格式化代码**：执行 `cargo fmt` 对代码进行格式化。


