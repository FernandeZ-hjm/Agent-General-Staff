#!/usr/bin/env node
// Prevent Claude from ending a turn after printing raw tool-call markup as text.

const fs = require('fs');

function readStdin() {
  try {
    return fs.readFileSync(0, 'utf8');
  } catch {
    return '';
  }
}

function parseJson(text) {
  if (!text.trim()) return {};
  try {
    return JSON.parse(text);
  } catch {
    return {};
  }
}

function lastAssistantText(input) {
  const value = input.last_assistant_message || input.lastAssistantMessage || '';
  if (typeof value === 'string') return value;
  if (Array.isArray(value)) {
    return value
      .map((part) => {
        if (typeof part === 'string') return part;
        if (part && typeof part.text === 'string') return part.text;
        return '';
      })
      .join('\n');
  }
  return '';
}

function hasRawToolCallLeak(text) {
  return /<invoke\s+name=["'][^"']+["']\s*>/i.test(text)
    || /<parameter\s+name=["'][^"']+["']\s*>/i.test(text)
    || /<\/invoke>/i.test(text);
}

const input = parseJson(readStdin());
const text = lastAssistantText(input);

if (hasRawToolCallLeak(text)) {
  const hookEventName = input.hook_event_name || input.hookEventName || 'Stop';
  process.stdout.write(JSON.stringify({
    suppressOutput: true,
    hookSpecificOutput: {
      hookEventName,
      additionalContext: [
        'The previous assistant message printed raw tool-call markup such as <invoke> as user-visible text.',
        'Continue now and perform the intended action with a real tool call instead of writing tool markup in prose.',
        'Do not ask the user to continue.'
      ].join(' ')
    }
  }));
}
