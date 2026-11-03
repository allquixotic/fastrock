# Fastrock 0.4.0-rc.9

- The conversation Provider picker hides incomplete configurations. It checks nonblank credentials, model configuration and endpoints locally, retaining existing Bedrock AWS profiles, SSO and credential helpers. Provider setup remains available in Settings. Opening the picker does not run helpers or probe a model.
- Model controls and the model button are left-aligned and fit their contents. Speed follows Thinking Effort. App-owned interface text uses functional labels without backend branding; configuration files and credentials remain in place.
- Bedrock Responses profiles can select Standard or Ultrafast for GPT-6.1 Sol and GPT-6 Astra on supported routes. Choices persist per conversation, including restoration; model/provider changes use the newly selected configuration. Luna and GPT-6 Sol are Standard-only. AWS does not document Fast/Priority for these models.
- Named profiles expose an optional API request path. For Bedrock Responses use `openai/v1/responses`, retaining the existing endpoint, credentials and model/inference profile ID. Existing Chat Completions and Converse configurations are unchanged; their speed control remains Standard-only with the pinned dependency.

Ultrafast has premium pricing (six times Standard in the current AWS model cards). See [Bedrock speed support](https://github.com/allquixotic/fastrock/blob/v0.4.0-rc.9/docs/BEDROCK-SPEED.md) for exact routes and integration limits.

Windows x64 ZIP. No separate backend installation is required. This is a release candidate; final live AWS profile acceptance remains with the user.
