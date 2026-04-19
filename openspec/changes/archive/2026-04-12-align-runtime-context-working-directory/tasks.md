## 1. Prompt context source alignment

- [ ] 1.1 Update request builder prompt composition to derive the primary working directory from `config.builtin.allowed_roots.first()` with fallback to `std::env::current_dir()`
- [ ] 1.2 Pass the full configured builtin allowed roots into `RuntimeContextRenderer::render()` as workspace roots

## 2. Verification

- [ ] 2.1 Add tests verifying runtime context uses the first configured allowed root as working directory
- [ ] 2.2 Add tests verifying runtime context includes all configured roots as workspace roots
- [ ] 2.3 Add tests verifying fallback to process current directory when no roots are configured
- [ ] 2.4 Run prompt composition and built-in tool related test suites to confirm prompt context and tool policy remain aligned
