# Bedrock speed selection

Verified against AWS model cards on 2026-11-03. The Speed control follows Provider, Mode, Model and Thinking Effort. It changes the session's `ModelConfig.request_params.service_tier` using the public SessionManager API in unmodified Goose 1.57.0. No proxy, provider process, global environment change or endpoint rewrite is involved.

| Model | Bedrock Runtime Responses | Mantle Responses | Choices |
|---|---|---|---|
| GPT-6.1 Sol | `us.openai.gpt-6.1-sol` or `global.openai.gpt-6.1-sol` | `openai.gpt-6.1-sol` in `us-east-1` | Standard, Ultrafast |
| GPT-6 Astra | `us.openai.gpt-6-astra` or `global.openai.gpt-6-astra` | `openai.gpt-6-astra` in `us-east-1` | Standard, Ultrafast |
| GPT-6 Luna / GPT-6 Sol | Configured supported route | Configured supported route | Standard |

US profiles require a documented US/Canada source region. The cached Sol 6.1 matrix uses its published US/Canada source regions; Astra global profiles also cover the other source regions listed in its model card. Unknown models, endpoints and routes fail closed to Standard. Mantle Oregon is Standard-only. Fast/Priority is not documented for these models; Fastrock does not send unsupported tiers. Ultrafast currently costs six times Standard.

Sources: [Sol 6.1](https://docs.aws.amazon.com/bedrock/latest/userguide/model-card-openai-gpt-6-1-sol.html), [Astra](https://docs.aws.amazon.com/bedrock/latest/userguide/model-card-openai-gpt-6-astra.html), [Luna](https://docs.aws.amazon.com/bedrock/latest/userguide/model-card-openai-gpt-6-luna.html), [Sol 6](https://docs.aws.amazon.com/bedrock/latest/userguide/model-card-openai-gpt-6-sol.html).

## Configuring Responses

Use an existing custom OpenAI-engine profile with an AWS HTTPS endpoint and `base_path: openai/v1/responses`. The named-profile editor exposes this optional API request path. Runtime base URL is `https://bedrock-runtime.REGION.amazonaws.com/openai/v1`; Mantle is `https://bedrock-mantle.us-east-1.api.aws/openai/v1`. Preserve your existing credentials and exact inference profile ID. Externally managed command-auth profiles remain externally editable; Fastrock does not rewrite them.

Pinned Goose derives Chat Completions from `/openai/v1` unless the declarative profile explicitly selects Responses. Its built-in Bedrock route uses Converse for these model IDs and cannot pass through this Responses setting. Those routes remain Standard-only in the picker. Selecting Speed never changes API families or regions behind the user's back.

`default` selects Standard; `ultrafast` selects Ultrafast. Other model parameters, including reasoning effort, remain intact. Existing saved unsupported/custom tier parameters are not silently rewritten; the picker leaves the selection blank until the user chooses a supported value. No custom speed choices are accepted.

## Provider readiness

Conversation choices require local model and credential configuration. API keys containing only whitespace do not count. Explicitly disabled entries, missing endpoints, missing models and missing helper executables are hidden. Command authentication is accepted without running the command; AWS shared profiles, source-role chains, SSO, web identity and container credential sources are recognized. Expiry, permissions and network reachability are resolved when the provider is used, not by paid/network probes when opening the picker. Settings continues to show supported engines for setup.
