# History

Nodes for updating the chat history

## Mask History

- Limits the number of messages the agent can see in a chat
- Non-destructive: can be reversed by removing the mask
- Remove mask by using another Mask node with limit >= 100

## Create Message

This node can create message objects from text and optionally include images.
Images can be supplied as URL references or image objects created by [media nodes](./media.md).

Message objects can be used as inputs to [Chat](./agent.md#chat) and
[Structured](./agent.md#structured-output) nodes or be used to [Extend History](#extend-history).

## Extend History

- Manually add messages to a conversation
- Not necessary when using Chat, Structured, Invoke Tools, etc
- But gives you more control over formatting

## Side Chat

- Creates an anonymous branch that merges back into parent
- Creates traceability without polluting main conversation
- Only start and end messages of side chat part of main conversation
- Remainder can be viewed by expanding the collapsible sections of chat tab
