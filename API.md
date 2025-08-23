# API Documentation

## Blueprint Management

### Update Blueprint

**Endpoint:** `POST /xrpc/tools.graze.ifthisthenat.updateBlueprint`

Creates or updates a blueprint with its nodes and evaluation order.

**Request Body:**
```json
{
  "aturi": "at://did:plc:example/tools.graze.ifthisthenat.blueprint.definition/abc123",
  "did": "did:plc:example",
  "node_order": [
    "at://did:plc:example/tools.graze.ifthisthenat.node.definition/node1",
    "at://did:plc:example/tools.graze.ifthisthenat.node.definition/node2",
    "at://did:plc:example/tools.graze.ifthisthenat.node.definition/node3"
  ],
  "nodes": [
    {
      "aturi": "at://did:plc:example/tools.graze.ifthisthenat.node.definition/node1",
      "node_type": "entry",
      "payload": {
        "input": "data"
      }
    },
    {
      "aturi": "at://did:plc:example/tools.graze.ifthisthenat.node.definition/node2",
      "node_type": "transform",
      "payload": {
        "transformation": "logic"
      }
    },
    {
      "aturi": "at://did:plc:example/tools.graze.ifthisthenat.node.definition/node3",
      "node_type": "transform",
      "payload": {
        "output": {"val": ["data"]}
      }
    }
  ]
}
```

**Notes:**
- The `node_order` array defines the sequence in which nodes will be evaluated by the datalogic engine
- Node AT-URIs in `node_order` should correspond to nodes defined in the `nodes` array
- If `aturi` is omitted from a node, a new AT-URI will be generated automatically

### Get Blueprint

**Endpoint:** `GET /xrpc/tools.graze.ifthisthenat.getBlueprint`

Retrieves a blueprint with its nodes and evaluation order.

**Query Parameters:**
- `aturi` (required): The AT-URI of the blueprint to retrieve

**Response:**
```json
{
  "blueprint": {
    "aturi": "at://did:plc:example/tools.graze.ifthisthenat.blueprint.definition/abc123",
    "did": "did:plc:example",
    "node_order": [
      "at://did:plc:example/tools.graze.ifthisthenat.node.definition/node1",
      "at://did:plc:example/tools.graze.ifthisthenat.node.definition/node2",
      "at://did:plc:example/tools.graze.ifthisthenat.node.definition/node3"
    ],
    "created_at": "2024-01-01T00:00:00Z"
  },
  "nodes": [
    {
      "aturi": "at://did:plc:example/tools.graze.ifthisthenat.node.definition/node1",
      "blueprint": "at://did:plc:example/tools.graze.ifthisthenat.blueprint.definition/abc123",
      "node_type": "entry",
      "payload": {
        "input": "data"
      },
      "created_at": "2024-01-01T00:00:00Z"
    }
  ]
}
```

## Node Types

- **jetstream_entry**: Entry point nodes that filter Jetstream events. If the condition fails, blueprint evaluation stops.
- **webhook_entry**: Entry point nodes that filter webhook requests. If the condition fails, blueprint evaluation stops.
- **condition**: Conditional nodes that use DataLogic expressions to return boolean results for flow control. If the condition returns false, blueprint evaluation stops.
- **transform**: Transformation nodes that process and reshape data using DataLogic expressions, or perform actions

## Evaluation Order

The `node_order` field in blueprints determines the sequence of node evaluation. The datalogic engine will process nodes in the specified order, passing data between them according to their defined connections and transformations.

## Blueprint Evaluation Engine

The blueprint engine uses the `datalogic-rs` crate to evaluate node payloads. Each node type has specific behavior:

### Entry Match Nodes
These nodes act as filters/guards. They evaluate a DataLogic expression against the input data and return a boolean result. If the result is false, the entire blueprint evaluation stops.

Example payload:
```json
{
  "==": [
    {"val": ["commit", "collection"]},
    "app.bsky.feed.post"
  ]
}
```

### Transform Nodes
These nodes transform the data by evaluating a DataLogic template. The result becomes the new data for subsequent nodes.

Example payload:
```json
{
  "$type": "app.bsky.feed.like",
  "createdAt": {
    "datetime": [{"now": []}]
  },
  "subject": {
    "uri": {
      "cat": [
        "at://",
        {"val": ["did"]},
        "/app.bsky.feed.post/",
        {"val": ["commit", "rkey"]}
      ]
    }
  }
}
```

### Example Input/Output

Given this input:
```json
{
  "commit": {
    "cid": "bafyreiapl6czyg3vmiwcrs53zy4n2ilh7hnczklrp3jhwvfkztgq4fimj4",
    "collection": "app.bsky.feed.post",
    "rkey": "3lwrimho2vk24"
  },
  "did": "did:plc:cbkjy5n7bk3ax2wplmtjofq2"
}
```

A transform node with the above payload would output:
```json
{
  "$type": "app.bsky.feed.like",
  "createdAt": "2025-08-22T18:17:48.121Z",
  "subject": {
    "cid": "bafyreiapl6czyg3vmiwcrs53zy4n2ilh7hnczklrp3jhwvfkztgq4fimj4",
    "uri": "at://did:plc:cbkjy5n7bk3ax2wplmtjofq2/app.bsky.feed.post/3lwrimho2vk24"
  }
}
```