# Fastrock 0.5.6

Fastrock now embeds Goose 1.60.2. Existing Goose profiles and credentials carry over.

GPT-6.2 Sol and Astra receive instructions to carry authorized tasks through verification, report progress during longer work, and explain the result and any blockers. Fastrock applies these instructions to recognized model IDs on OpenAI, Amazon Bedrock, and named OpenAI-compatible profiles. Switching to another model removes the instructions. Your saved reasoning effort, permissions, and custom prompts still apply.

Goose now opens sessions saved in **Chat** mode in **Approve** mode. It asks for approval before tool calls.

This release ships as a signed Windows x64 ZIP. Use **Quit** in the previous version’s tray menu before launching 0.5.6.
