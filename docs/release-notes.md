# Fastrock 0.5.4

- Use **MAI Code 2 Flash** through Microsoft Foundry with your work Azure CLI sign-in.
  In **Settings → Goose → Named provider profiles**, choose **Microsoft Foundry
  (Azure CLI SSO)** and enter the project endpoint, deployment name, and Azure
  subscription ID. Fastrock obtains short-lived credentials on demand.
- Microsoft offers Flash in a limited-access Foundry preview. The full MAI Code 2
  is available in GitHub Copilot. Ask your Azure team to confirm preview access,
  project permissions, and where the deployment processes inference.
- Use **GPT-6.2 Sol** and **GPT-6.2 Astra** on Amazon Bedrock with your existing
  Bedrock profile. Enter the model or inference profile ID from the Bedrock
  console in **Goose options → Use model ID**.

[Foundry setup guide](https://github.com/allquixotic/fastrock/blob/v0.5.4/docs/goose-acp.md#microsoft-foundry-and-azure-cli)
[Bedrock setup guide](https://github.com/allquixotic/fastrock/blob/v0.5.4/docs/goose-acp.md#gpt-62-on-amazon-bedrock)

This release ships as a signed Windows x64 ZIP. Use **Quit** in the previous
version's tray menu before launching 0.5.4.
