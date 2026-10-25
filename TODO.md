# TODO

## OpenAI-совместимый провайдер

Провайдер для любого сервера с Chat Completions API (OpenRouter, Ollama, vLLM, LM Studio, DeepSeek и т.п.), как в Roo Code. Сейчас `openai` ходит только в `https://api.openai.com/v1/responses` (Responses API), адрес не настраивается.

- [ ] Маршрут `openai-compat:<model>`.
- [ ] Настройки: `OPENAI_COMPAT_BASE_URL`, `OPENAI_COMPAT_API_KEY`; возможно, несколько именованных эндпоинтов.
- [ ] Перевод Anthropic Messages → `/chat/completions`: сообщения, system, `tools` / `tool_calls`, изображения.
- [ ] Стриминг: SSE Chat Completions → Anthropic-события, включая сборку `tool_calls` по дельтам.
- [ ] `reasoning_content` (DeepSeek и др.) → thinking-блоки.
- [ ] Регистрация в routing и `CLAUDE_BRIDGE_DISABLED_PROVIDERS`.
- [ ] UI: ключ и адрес на вкладке «Аккаунты», переключатель на вкладке «Провайдеры».
- [ ] README (ru/en).
