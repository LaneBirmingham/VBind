// How to hook an AI coding agent up to VBind: MCP for agents that support it,
// the `vbind` command line for any agent that can run commands.

import { status } from "../state";
import { CopyBox, Modal } from "./bits";

export function ConnectAgent({ onClose }: { onClose: () => void }) {
  const cli = status.value?.cli ?? "vbind.exe";
  const q = `"${cli}"`;
  const json = JSON.stringify({ mcpServers: { vbind: { command: cli, args: ["mcp"] } } }, null, 2);
  return (
    <Modal title="Connect your AI agent" onClose={onClose}>
      <p class="muted" style={{ margin: 0 }}>
        VBind is built to be driven by an AI agent. Once connected, your agent can see your controllers, ask you to press buttons, write the
        mapping and test it, and it posts its progress in this window.
      </p>

      <div>
        <div class="section-title">Claude Code</div>
        <CopyBox text={`claude mcp add --scope user vbind -- ${q} mcp`} />
      </div>
      <div>
        <div class="section-title">Codex</div>
        <CopyBox text={`codex mcp add vbind -- ${q} mcp`} />
      </div>
      <div>
        <div class="section-title">Other MCP apps (Cursor, VS Code, Windsurf, Claude Desktop...)</div>
        <CopyBox text={json} />
      </div>
      <div>
        <div class="section-title">Any agent that can run commands</div>
        <p class="small muted" style={{ margin: "0 0 6px" }}>
          No MCP needed. Tell the agent to run this and follow what it says:
        </p>
        <CopyBox text={`${q} guide`} />
      </div>
      <div>
        <div class="section-title">Then ask it something like</div>
        <CopyBox text="Use VBind to look at the game controllers I have plugged in and map them to a virtual Xbox controller for my game. Show me your plan in the VBind window first." />
      </div>
    </Modal>
  );
}
