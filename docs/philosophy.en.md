# The Five Articles: The Design Philosophy of AGS

The README compresses the five articles into a table. A table is easy to remember, but it won't tell you why. This document fills in the why — what kind of accident forced each article into being, and which gate it became inside AGS.

The order doesn't matter. These weren't deduced from some abstract principle. They're what an AI-coding newcomer picked up, one at a time, after tripping over the same handful of problems for a month.

## I · Don't Trust a Single AI or a Single Tool

The strongest AI coding tools right now are roughly Codex, Claude Code, Gemini CLI, and Cursor. They're all strong, but strong at different things: coding style, execution habits, capability limits, how they misjudge risk, cost structure — all different. And versions move fast: the build that felt great this week can turn sour the next.

So I trust "one model to rule them all" less and less. In my own flow, planning, review, and big bug fixes go to one class of model; the concrete execution goes to another; and after delivery I bring in a third to sign off — because a model often can't catch the problems in the code it just delivered.

AGS doesn't replace any single agent. It gives them a shared engineering order: who plans, who executes, who reviews, who may touch files, who is read-only, and when to stop and wait for a human. One install per machine, AGS governs, and every agent works under the same rules.

## II · AI Can't Fully Understand Human Speech

A prompt is, at bottom, chat language. Chinese is especially tricky — the same word can mean opposite things in different contexts. Plenty of agents can "optimize the prompt" now, but optimize all you want, natural language is still natural language. It is not an engineering contract, and it is not a machine protocol.

So a human's intent shouldn't be handed straight to the executing agent. There has to be a translation step in between. First preflight, so the agent knows what this repository is, what rules apply, what history it carries, and what it must stop and ask about. Then solution formation, working with the human to pin down the goal, the boundaries, the non-goals. Only after the user confirms does a task card get generated; the card is then resolved into an execution policy; and only past the gate does real work begin.

In one line: a prompt is chat language, the task card is the engineering contract. The card must spell out the goal, background, non-goals, permission mode, execution boundaries, verification method, and delivery format. It isn't a form for a human to fill in — it's a human's intent, calibrated into a task an agent can execute reliably.

## III · Execution Is Not a Straight Line

An AI's execution isn't a straight line. Sometimes it's brilliant, sometimes ordinary, sometimes distracted, sometimes confidently talking nonsense. So you can't judge it by the last line — "I finished."

A workflow you can actually trust has to keep the trail: what task card it received, what permissions it got, whether the gate passed, which files it changed, which verification commands it ran, what the results were, and whether a receipt came out the other end.

AGS's goal isn't a model that never errs — that's not realistic. Its goal is that errors never happen quietly. Even when it slips, a human can trace it: where it slipped, why, what changed, what didn't, what got verified, and what's still uncertain.

You can't control every output of a component that drifts, but you can make every output recorded and observed. An error you can see is an error you can fix.

## IV · Human Judgment Deserves to Be Saved

The longer I work on a project, the more I feel the valuable thing isn't any single model output — it's the human judgment at the solution and architecture stage: why this feature can't be built that way; why this boundary has to be hard; why this task is read-only; why this plan should be split in two; why this pothole must be avoided next time.

Leave that in the chat log and it's gone fast. AGS catches it with the memory capsule: project profile, context memory, task archive, delivery records, key decisions, verification results, open items, risk notes. Open a fresh chat, and as long as it happened inside this project, the agent can recall it from here — not from the thin context summary the model carries on its own.

One of the biggest losses in AI coding is context loss. You taught it once, you switch chats, and it hits the same pothole again like it has amnesia. What the memory capsule does is let experience escape the chat log and become a project asset.

## V · Mix Your Models, Work Without Fatigue

Top-tier foreign models really are good, but expensive. Domestic models are cheap and plentiful, but unstable when fully unsupervised. The value of AGS is putting the cheaper model inside an engineering process, so it executes under a clear task, clear boundaries, and clear acceptance criteria. Top-tier models make the key calls, cheaper models do the bulk of the concrete work, AGS keeps the whole run in line; after delivery, a stronger model sweeps for what's missing.

This isn't frugality for its own sake — it's a reality AI coding will have to face. Plenty of platforms are tightening quotas; you can't run every task on the most expensive model the whole way through.

Behind this article is a plain engineering instinct: you can't make a cheap component expensive, but you can use a deterministic process to make it deliver deterministic results inside deterministic boundaries. Model capability fluctuates; the engineering process carries the stability — and nowhere is that line more concrete than here.

## The Old Discipline Behind This Order

By now you've probably seen it: not one of these five articles is a new invention of the AI era.

Take an imprecise component that drifts and carries noise, put it inside a system with feedback, constraints, and records, and make the whole thing deliver usable results reliably — that's what engineering cybernetics spelled out back in 1954. When Qian Xuesen wrote *Engineering Cybernetics*, he was dealing with the gyroscopes and servomotors of a missile; today we're dealing with a large model that writes code and also talks nonsense. The component under control changed; the control idea didn't: you don't chase a part that never fails, you design a loop that tolerates failure.

AGS just moves that old idea onto the floor of AI coding. The task card is the setpoint, the gate is the constraint, verification and the receipt are the feedback, and the memory capsule lets the system remember the road it has already walked. Call it innovation if you like; it's closer to a belated return to common sense.

Back to the [README](../README.en.md) to see how these five become concrete commands.
