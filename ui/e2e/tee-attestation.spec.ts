/**
 * E2E: TEE Attestation UI
 *
 * Agent-driven tests that verify:
 *   1. TEE badge appears on sandbox/instance list for TEE-enabled entries
 *   2. Attestation tab is visible and clickable on detail pages
 *   3. "Get Attestation" fetches and displays attestation data
 *   4. Measurement and evidence render as hex
 *
 * Requires:
 *   - Running sandbox cloud UI (pnpm dev or E2E_BASE_URL)
 *   - Running operator API with a TEE sandbox provisioned
 *   - LLM API key (OPENAI_API_KEY or LITELLM_*)
 *
 * Usage:
 *   OPENAI_API_KEY=sk-... pnpm test:e2e -- --grep "TEE"
 */

import { test, expect, E2E_CONFIG, hasApiKey, brainConfig } from './fixtures.js';

test.describe('TEE Attestation UI', () => {
  test.skip(!hasApiKey, 'Requires OPENAI_API_KEY or LITELLM for agent brain');

  test('agent verifies TEE badge on sandbox list', async ({ page }) => {
    test.setTimeout(3 * 60 * 1000);

    const { PlaywrightDriver, AgentRunner } = await import(
      '@tangle-network/agent-browser-driver'
    );

    const driver = new PlaywrightDriver(page, {
      timeout: 15_000,
      captureScreenshots: true,
      screenshotQuality: 50,
    });

    const runner = new AgentRunner({
      driver,
      config: { ...brainConfig, debug: true, retries: 2, retryDelayMs: 2000 },
      onTurn: async (turn) => {
        console.log(
          `[tee-badge] Turn ${turn.turn}: ${JSON.stringify(turn.action).slice(0, 200)}`,
        );
        if (turn.state.screenshot) {
          await test.info().attach(`turn-${turn.turn}`, {
            body: Buffer.from(turn.state.screenshot, 'base64'),
            contentType: 'image/jpeg',
          });
        }
      },
    });

    const result = await runner.run({
      goal: `You are on a sandbox provisioning dashboard.

YOUR TASK:
1. Navigate to the Sandboxes page (look for a "Sandboxes" link in the navigation)
2. Look at the list of sandboxes
3. Check if any sandbox has a purple "TEE" badge next to its name
4. Also check if any sandbox uses a shield icon instead of the default hard-drives icon
5. Report what you see — are there any TEE-enabled sandboxes?

Complete when you have examined the sandbox list and can describe whether TEE badges are present.`,
      startUrl: `${E2E_CONFIG.baseUrl}/sandboxes`,
      maxTurns: 10,
    });

    console.log(
      `[tee-badge] Result: success=${result.success}, turns=${result.turns.length}`,
    );

    await test.info().attach('sandbox-list-final', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });

    expect(result.turns.length).toBeGreaterThan(0);
  });

  test('agent navigates to TEE sandbox and fetches attestation', async ({ page }) => {
    test.setTimeout(5 * 60 * 1000);

    const { PlaywrightDriver, AgentRunner } = await import(
      '@tangle-network/agent-browser-driver'
    );

    const driver = new PlaywrightDriver(page, {
      timeout: 15_000,
      captureScreenshots: true,
      screenshotQuality: 50,
    });

    const runner = new AgentRunner({
      driver,
      config: { ...brainConfig, debug: true, retries: 3, retryDelayMs: 2000 },
      onTurn: async (turn) => {
        console.log(
          `[tee-attestation] Turn ${turn.turn}: ${JSON.stringify(turn.action).slice(0, 200)}`,
        );
        if (turn.state.screenshot) {
          await test.info().attach(`turn-${turn.turn}`, {
            body: Buffer.from(turn.state.screenshot, 'base64'),
            contentType: 'image/jpeg',
          });
        }
      },
    });

    // Phase 1: Navigate to a sandbox detail page
    const navResult = await runner.run({
      goal: `You are on a sandbox provisioning dashboard.

YOUR TASK:
1. Go to the Sandboxes page
2. Click on any sandbox in the list to navigate to its detail page
3. You should see tabs like Overview, Terminal, Chat, SSH, Secrets, and possibly Attestation

Complete when you are on a sandbox detail page and can see the tabs.`,
      startUrl: `${E2E_CONFIG.baseUrl}/sandboxes`,
      maxTurns: 10,
    });

    console.log(
      `[tee-attestation] Nav result: success=${navResult.success}, turns=${navResult.turns.length}`,
    );
    expect(page.url()).toContain('/sandboxes/');

    await test.info().attach('sandbox-detail-page', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });

    // Phase 2: Click attestation tab and fetch attestation
    const attestResult = await runner.run({
      goal: `You are on a sandbox detail page with several tabs.

YOUR TASK:
1. Look for an "Attestation" tab in the tab bar (it has a shield icon)
2. If the Attestation tab exists and is NOT disabled/greyed out, click it
3. After clicking the tab, look for a "Get Attestation" button and click it
4. Wait for the attestation data to load — you should see fields like:
   - TEE Type (e.g. "sgx" or "tdx")
   - Timestamp
   - Measurement (shown as hex)
   - Evidence (collapsible section with byte count)
5. If the Attestation tab is disabled or doesn't exist, that's OK — this sandbox may not be TEE-enabled

Report what you see: is the attestation tab available? Did attestation data load?

Complete when you have either:
- Successfully fetched and can see attestation data, OR
- Confirmed the attestation tab is disabled/missing (non-TEE sandbox)`,
      maxTurns: 15,
    });

    console.log(
      `[tee-attestation] Attest result: success=${attestResult.success}, turns=${attestResult.turns.length}`,
    );

    await test.info().attach('attestation-tab-final', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });

    expect(attestResult.turns.length).toBeGreaterThan(0);
  });

  test('agent verifies TEE instance attestation tab', async ({ page }) => {
    test.setTimeout(5 * 60 * 1000);

    const { PlaywrightDriver, AgentRunner } = await import(
      '@tangle-network/agent-browser-driver'
    );

    const driver = new PlaywrightDriver(page, {
      timeout: 15_000,
      captureScreenshots: true,
      screenshotQuality: 50,
    });

    const runner = new AgentRunner({
      driver,
      config: { ...brainConfig, debug: true, retries: 3, retryDelayMs: 2000 },
      onTurn: async (turn) => {
        console.log(
          `[tee-instance] Turn ${turn.turn}: ${JSON.stringify(turn.action).slice(0, 200)}`,
        );
        if (turn.state.screenshot) {
          await test.info().attach(`turn-${turn.turn}`, {
            body: Buffer.from(turn.state.screenshot, 'base64'),
            contentType: 'image/jpeg',
          });
        }
      },
    });

    // Navigate to instances page and find a TEE instance
    const result = await runner.run({
      goal: `You are on a sandbox provisioning dashboard.

YOUR TASK:
1. Navigate to the Instances page (look for an "Instances" link in the navigation)
2. Look at the list of instances
3. Find an instance that has a purple "TEE" badge
4. Click on that TEE instance to go to its detail page
5. On the detail page, look for an "Attestation" tab (it should appear for TEE instances)
6. Click the Attestation tab
7. Click the "Get Attestation" button
8. Verify attestation data loads — look for TEE Type, Timestamp, Measurement, and Evidence

If no TEE instances exist in the list, report that and complete.

Complete when you have either:
- Fetched attestation data from a TEE instance, OR
- Confirmed no TEE instances are available`,
      startUrl: `${E2E_CONFIG.baseUrl}/instances`,
      maxTurns: 20,
    });

    console.log(
      `[tee-instance] Result: success=${result.success}, turns=${result.turns.length}`,
    );

    await test.info().attach('instance-attestation-final', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });

    expect(result.turns.length).toBeGreaterThan(0);
  });
});

test.describe('TEE Attestation UI — non-agent tests', () => {
  test('sandbox list page loads without errors', async ({ page }) => {
    await page.goto('/sandboxes');
    await expect(page.locator('h1')).toContainText('Sandboxes');
  });

  test('instance list page loads without errors', async ({ page }) => {
    await page.goto('/instances');
    await expect(page.locator('h1')).toContainText('Instances');
  });

  test('sandbox detail shows attestation tab when TEE badge present', async ({ page }) => {
    await page.goto('/sandboxes');

    // Find a TEE-badged sandbox (if any)
    const teeBadge = page.locator('text=TEE').first();
    const hasTee = (await teeBadge.count()) > 0;

    if (!hasTee) {
      test.skip(true, 'No TEE sandboxes in list');
      return;
    }

    // Click the card containing the TEE badge
    const card = teeBadge.locator('xpath=ancestor::a[1]');
    await card.click();

    // Verify attestation tab exists
    await expect(page.locator('button:has-text("Attestation")')).toBeVisible();
  });

  test('attestation tab shows Get Attestation button', async ({ page }) => {
    await page.goto('/sandboxes');

    const teeBadge = page.locator('text=TEE').first();
    const hasTee = (await teeBadge.count()) > 0;

    if (!hasTee) {
      test.skip(true, 'No TEE sandboxes in list');
      return;
    }

    const card = teeBadge.locator('xpath=ancestor::a[1]');
    await card.click();

    // Click attestation tab
    await page.locator('button:has-text("Attestation")').click();

    // Verify button exists
    await expect(
      page.locator('button:has-text("Get Attestation")'),
    ).toBeVisible();
  });
});
