# Blueprint Guide

This comprehensive guide covers creating, configuring, and managing blueprints for the ifthisthenat automation system. Blueprints are automated workflows that respond to events on the AT Protocol network and perform actions.

## Table of Contents

- [Blueprint Overview](#blueprint-overview)
- [Blueprint Structure](#blueprint-structure)
- [Node Types](#node-types)
- [Entry Nodes](#entry-nodes)
- [Processing Nodes](#processing-nodes)
- [Action Nodes](#action-nodes)
- [Complex Examples](#complex-examples)
- [Prototype System](#prototype-system)
- [Best Practices](#best-practices)
- [Testing and Debugging](#testing-and-debugging)

## Blueprint Overview

A blueprint consists of:

1. **Entry Node**: Defines what triggers the blueprint (jetstream_entry, webhook_entry, or periodic_entry)
2. **Processing Nodes**: Transform, filter, or validate the data (condition, transform, facet_text)
3. **Action Node**: Performs the final action (publish_record, publish_webhook, debug_action)

Blueprints are evaluated sequentially, with each node's output becoming the next node's input. If any node fails, the blueprint evaluation stops.

## Blueprint Structure

### Basic Blueprint Format

```json
{
  "aturi": "at://did:plc:user/tools.graze.ifthisthenat.blueprint/blueprint-id",
  "nodes": [
    {
      "node_type": "jetstream_entry",
      "configuration": { /* node-specific config */ },
      "payload": { /* DataLogic expression or static data */ }
    },
    {
      "node_type": "condition",
      "configuration": {},
      "payload": { /* DataLogic boolean expression */ }
    },
    {
      "node_type": "publish_record",
      "configuration": {
        "collection": "app.bsky.feed.post",
        "did": "did:plc:your-did"
      },
      "payload": { /* DataLogic expression for record data */ }
    }
  ]
}
```

### AT Protocol Event Examples

The system receives real-time events from the AT Protocol network. Here are common event structures:

#### Bluesky Feed Post Event
```json
{
  "commit": {
    "cid": "bafyreiga4g6vrd3lj557afdyec4xwld4gs2t3u47y4ybggvnc7i24udcnq",
    "collection": "app.bsky.feed.post",
    "operation": "create",
    "record": {
      "$type": "app.bsky.feed.post",
      "createdAt": "2025-09-04T03:19:50.142Z",
      "langs": ["en"],
      "text": "Who's interested in automation?"
    },
    "rev": "3lxy6sv2j5k2b",
    "rkey": "3lxy6suve4c2y"
  },
  "did": "did:plc:tgudj2fjm77pzkuawquqhsxm",
  "kind": "commit",
  "time_us": 1756955990660025
}
```

#### Bluesky Like Event
```json
{
  "commit": {
    "collection": "app.bsky.feed.like",
    "operation": "create",
    "record": {
      "$type": "app.bsky.feed.like",
      "createdAt": "2025-09-04T03:20:36.439Z",
      "subject": {
        "cid": "bafyreiga4g6vrd3lj557afdyec4xwld4gs2t3u47y4ybggvnc7i24udcnq",
        "uri": "at://did:plc:tgudj2fjm77pzkuawquqhsxm/app.bsky.feed.post/3lxy6suve4c2y"
      }
    }
  },
  "did": "did:plc:cbkjy5n7bk3ax2wplmtjofq2",
  "kind": "commit"
}
```

## Node Types

### Entry Nodes

Entry nodes must be the first node in a blueprint and define what triggers the evaluation.

#### Available Entry Node Types
- `jetstream_entry` - AT Protocol events from Jetstream
- `webhook_entry` - HTTP webhook requests
- `periodic_entry` - Scheduled/cron-based execution

#### Processing Node Types
- `condition` - Filter events based on boolean expressions
- `transform` - Transform data using DataLogic expressions
- `facet_text` - Process text to extract mentions and links

#### Action Node Types
- `publish_record` - Publish records to AT Protocol repositories
- `publish_webhook` - Send HTTP requests to external services
- `debug_action` - Log information for debugging

**Important**: Blueprints must have at least one action node.

## Entry Nodes

### Jetstream Entry

Triggers when events are received from the AT Protocol firehose.

#### Match by Collection
```json
{
  "node_type": "jetstream_entry",
  "configuration": {
    "collection": ["app.bsky.feed.post"]
  },
  "payload": true
}
```

#### Match by Identity (DID)
```json
{
  "node_type": "jetstream_entry",
  "configuration": {
    "did": ["did:plc:tgudj2fjm77pzkuawquqhsxm"]
  },
  "payload": true
}
```

#### Match Posts with Specific Content
```json
{
  "node_type": "jetstream_entry",
  "configuration": {
    "collection": ["app.bsky.feed.post"]
  },
  "payload": {
    "and": [
      {"==": [{"val": ["commit", "operation"]}, "create"]},
      {"exists": ["commit", "record", "text"]},
      {"or": [
        {"starts_with": [{"val": ["commit", "record", "text"]}, "#automation"]},
        {"ends_with": [{"val": ["commit", "record", "text"]}, "#ifthisthenat"]}
      ]}
    ]
  }
}
```

### Webhook Entry

Processes HTTP webhook requests to `/webhooks/{blueprint_record_key}`.

#### Basic Webhook Handler
```json
{
  "node_type": "webhook_entry",
  "configuration": {},
  "payload": true
}
```

#### Validate Webhook Structure
```json
{
  "node_type": "webhook_entry",
  "configuration": {},
  "payload": {
    "and": [
      {"exists": ["body", "event_type"]},
      {"exists": ["body", "data"]},
      {"in": [{"val": ["body", "event_type"]}, ["user_signup", "payment_success"]]}
    ]
  }
}
```

**Webhook Request Structure:**
```json
{
  "headers": {
    "content-type": "application/json",
    "authorization": "Bearer token"
  },
  "body": { /* POST body data */ },
  "query": { /* URL parameters */ },
  "method": "POST",
  "path": "/webhooks/blueprint-record-key"
}
```

### Periodic Entry

Generates events on a schedule using cron expressions.

#### Daily Status Report
```json
{
  "node_type": "periodic_entry",
  "configuration": {
    "cron": "0 0 9 * * *"  // Daily at 9 AM
  },
  "payload": {
    "event_type": "daily_summary",
    "timestamp": {"datetime": [{"now": []}]},
    "day": {"format_date": [{"now": []}, "dddd"]},
    "date": {"format_date": [{"now": []}, "YYYY-MM-DD"]}
  }
}
```

#### Cron Expression Format
```
MIN HOUR DAY MONTH WEEKDAY
```

**Examples:**
- `* * * * *` - Every minute
- `0 * * * *` - Every hour at minute 0
- `0 0 * * *` - Daily at midnight
- `*/5 * * * *` - Every 5 minutes
- `0 9-17 * * 1-5` - Every hour from 9am to 5pm on weekdays

**Special Strings:**
- `@hourly` - Run once every hour
- `@daily` - Run once a day at midnight
- `@weekly` - Run once a week on Sunday
- `@monthly` - Run once a month on the 1st

**Limitations:**
- Minimum interval: 30 seconds
- Maximum interval: 90 days
- All times are in UTC

## Processing Nodes

### Condition Nodes

Filter events based on boolean expressions using DataLogic.

#### Check Post Language and Length
```json
{
  "node_type": "condition",
  "configuration": {},
  "payload": {
    "and": [
      {"exists": ["commit", "record", "text"]},
      {">=": [{"length": {"val": ["commit", "record", "text"]}}, 10]},
      {"<=": [{"length": {"val": ["commit", "record", "text"]}}, 280]},
      {"or": [
        {"in": ["en", {"val": ["commit", "record", "langs"]}]},
        {"!": {"exists": ["commit", "record", "langs"]}}
      ]}
    ]
  }
}
```

#### Check if Event is Recent
```json
{
  "node_type": "condition",
  "configuration": {},
  "payload": {
    ">": [
      {"val": ["time_us"]},
      {"/": [{"-": [{"*": [{"now": []}, 1000]}, 86400000]}, 1000]}
    ]
  }
}
```

### Transform Nodes

Transform data using DataLogic expressions for dynamic content generation.

#### Extract Post Text and Author
```json
{
  "node_type": "transform",
  "configuration": {},
  "payload": {
    "post_text": {"val": ["commit", "record", "text"]},
    "author_did": {"val": ["did"]},
    "post_uri": {"cat": [
      "at://",
      {"val": ["did"]},
      "/app.bsky.feed.post/",
      {"val": ["commit", "rkey"]}
    ]},
    "created_time": {"val": ["commit", "record", "createdAt"]},
    "has_images": {"exists": ["commit", "record", "embed", "images"]},
    "lang": {"first": {"val": ["commit", "record", "langs"]}}
  }
}
```

#### Create Record with Conditional Content
```json
{
  "node_type": "transform",
  "configuration": {},
  "payload": {
    "record": {
      "$type": "app.bsky.feed.post",
      "createdAt": {"datetime": [{"now": []}]},
      "text": {"if": [
        {"==": [{"val": ["commit", "collection"]}, "app.bsky.feed.like"]},
        "Thanks for the like! ❤️",
        {"if": [
          {"==": [{"val": ["commit", "collection"]}, "app.bsky.feed.repost"]},
          "Thanks for the repost! 🔄",
          {"cat": ["New activity: ", {"val": ["commit", "collection"]}]}
        ]}
      ]},
      "langs": ["en"]
    }
  }
}
```

### Facet Text Nodes

Process text to extract mentions (@handles) and URLs, creating AT Protocol facets for rich text.

#### Basic Text Processing
```json
{
  "node_type": "facet_text",
  "configuration": {},
  "payload": {}
}
```

#### Custom Field Processing
```json
{
  "node_type": "facet_text",
  "configuration": {
    "field": "content"  // Process text from specific field
  },
  "payload": {}
}
```

**Input Example:**
```json
{
  "text": "Great discussion with @alice.bsky.social! More info at https://docs.example.com"
}
```

**Output Structure:**
```json
{
  "text": "Great discussion with @alice.bsky.social! More info at https://docs.example.com",
  "facets": [
    {
      "index": {"byteStart": 21, "byteEnd": 39},
      "features": [{"$type": "app.bsky.richtext.facet#mention", "did": "did:plc:alice123"}]
    },
    {
      "index": {"byteStart": 55, "byteEnd": 78},
      "features": [{"$type": "app.bsky.richtext.facet#link", "uri": "https://docs.example.com"}]
    }
  ]
}
```

## Action Nodes

### Publish Record

Publish records to AT Protocol repositories.

#### Publish Auto-Generated Like
```json
{
  "node_type": "publish_record",
  "configuration": {
    "collection": "app.bsky.feed.like",
    "did": "did:plc:your-did-here"
  },
  "payload": {"val": []}  // Uses data from previous node
}
```

#### Publish with Custom Record Key
```json
{
  "node_type": "publish_record",
  "configuration": {
    "collection": "community.lexicon.calendar.rsvp",
    "did": "did:plc:your-did-here",
    "rkey": {"cat": [
      {"val": ["commit", "rkey"]},
      "-rsvp"
    ]}
  },
  "payload": {"val": []}
}
```

### Publish Webhook

Send HTTP requests to external services.

#### Send to Analytics Service
```json
{
  "node_type": "publish_webhook",
  "configuration": {
    "url": "https://analytics.example.com/events",
    "timeout_ms": 5000,
    "headers": {
      "Authorization": "Bearer your-api-key",
      "Content-Type": "application/json"
    }
  },
  "payload": {"val": []}
}
```

#### Send with Retry Configuration (Queue Mode)
```json
{
  "node_type": "publish_webhook",
  "configuration": {
    "url": "https://api.critical-service.com/webhook",
    "max_retries": 5,
    "timeout_ms": 15000,
    "correlation_id": "transaction-456",
    "headers": {
      "Authorization": "Bearer token"
    },
    "tags": {
      "priority": "high",
      "environment": "production"
    }
  },
  "payload": {"val": []}
}
```

### Debug Action

Log data for debugging and development.

#### Basic Debug Logging
```json
{
  "node_type": "debug_action",
  "configuration": {},
  "payload": {}  // Logs current pipeline data
}
```

**Debug Output Format:**
- Node AT-URI
- Blueprint AT-URI
- Pretty-printed JSON of input data

## Complex Examples

### Auto-RSVP to Calendar Events

```json
{
  "nodes": [
    {
      "node_type": "jetstream_entry",
      "configuration": {
        "collection": ["community.lexicon.calendar.event"]
      },
      "payload": true
    },
    {
      "node_type": "condition",
      "configuration": {},
      "payload": {"==": [
        {"val": ["commit", "record", "locations", 0, "locality"]},
        "Dayton"
      ]}
    },
    {
      "node_type": "transform",
      "configuration": {},
      "payload": {
        "record": {
          "$type": "community.lexicon.calendar.rsvp",
          "createdAt": {"datetime": [{"now": []}]},
          "status": "community.lexicon.calendar.rsvp#going",
          "subject": {
            "cid": {"val": ["commit", "cid"]},
            "uri": {"cat": [
              "at://",
              {"val": ["did"]},
              "/",
              {"val": ["commit", "collection"]},
              "/",
              {"val": ["commit", "rkey"]}
            ]}
          }
        }
      }
    },
    {
      "node_type": "publish_record",
      "configuration": {
        "collection": "community.lexicon.calendar.rsvp",
        "did": "did:plc:your-did-here"
      },
      "payload": {"val": []}
    }
  ]
}
```

### Content Analysis Pipeline

```json
{
  "nodes": [
    {
      "node_type": "jetstream_entry",
      "configuration": {
        "collection": ["app.bsky.feed.post"],
        "did": ["did:plc:specific-user"]
      },
      "payload": true
    },
    {
      "node_type": "condition",
      "configuration": {},
      "payload": {"and": [
        {">": [{"length": {"val": ["commit", "record", "text"]}}, 10]},
        {"<": [{"length": {"val": ["commit", "record", "text"]}}, 300]},
        {"not": {"contains": [{"val": ["commit", "record", "text"]}, "RT @"]}}
      ]}
    },
    {
      "node_type": "transform",
      "configuration": {},
      "payload": {
        "analysis": {
          "post_text": {"val": ["commit", "record", "text"]},
          "word_count": {"length": {"split": [{"val": ["commit", "record", "text"]}, " "]}},
          "has_mentions": {"some": [
            {"extract_mentions": {"val": ["commit", "record", "text"]}},
            true
          ]},
          "priority_level": {"if": [
            {">": [{"val": ["metrics", "likeCount"]}, 100]},
            "high",
            {"if": [
              {">": [{"val": ["metrics", "likeCount"]}, 10]},
              "medium",
              "low"
            ]}
          ]}
        }
      }
    },
    {
      "node_type": "condition",
      "configuration": {},
      "payload": {"==": [{"val": ["analysis", "priority_level"]}, "high"]}
    },
    {
      "node_type": "publish_webhook",
      "configuration": {
        "url": "https://api.analytics.example.com/high-priority-posts",
        "headers": {
          "Authorization": "Bearer analytics-key",
          "X-Priority": "high"
        }
      },
      "payload": {"val": []}
    }
  ]
}
```

### Scheduled Daily Post with Rich Text

```json
{
  "nodes": [
    {
      "node_type": "periodic_entry",
      "configuration": {
        "cron": "0 9 * * *"  // Daily at 9 AM
      },
      "payload": {
        "day": {"format_date": [{"now": []}, "dddd"]},
        "date": {"format_date": [{"now": []}, "YYYY-MM-DD"]}
      }
    },
    {
      "node_type": "transform",
      "configuration": {},
      "payload": {
        "text": {"cat": [
          "Good morning! Today is ",
          {"val": ["day"]},
          ", ",
          {"val": ["date"]},
          ". Check out our website: https://example.com"
        ]}
      }
    },
    {
      "node_type": "facet_text",
      "configuration": {
        "field": "text"
      },
      "payload": {}
    },
    {
      "node_type": "transform",
      "configuration": {},
      "payload": {
        "record": {
          "$type": "app.bsky.feed.post",
          "text": {"val": ["text"]},
          "facets": {"val": ["facets"]},
          "createdAt": {"datetime": [{"now": []}]},
          "langs": ["en"]
        }
      }
    },
    {
      "node_type": "publish_record",
      "configuration": {
        "collection": "app.bsky.feed.post",
        "did": "did:plc:your-did-here"
      },
      "payload": {"val": []}
    }
  ]
}
```

## Prototype System

The prototype system allows creating reusable blueprint templates with placeholders.

### Prototype Structure

```json
{
  "aturi": "at://did:plc:user/tools.graze.ifthisthenat.prototype/daily-post",
  "name": "Daily Status Post",
  "description": "Posts a daily status update at a scheduled time",
  "nodes": [
    {
      "node_type": "periodic_entry",
      "configuration": {
        "cron": "$[SCHEDULE]"
      },
      "payload": {
        "event_type": "daily_post",
        "timestamp": {"datetime": [{"now": []}]}
      }
    },
    {
      "node_type": "transform",
      "payload": {
        "record": {
          "$type": "app.bsky.feed.post",
          "text": "$[MESSAGE]",
          "createdAt": {"val": ["timestamp"]}
        }
      }
    },
    {
      "node_type": "publish_record",
      "configuration": {
        "did": "$[USER_DID]",
        "collection": "app.bsky.feed.post"
      }
    }
  ],
  "placeholders": [
    {
      "id": "USER_DID",
      "label": "Your DID",
      "description": "The DID to publish posts as",
      "value_type": "did",
      "required": true,
      "example": "did:plc:yourhandle"
    },
    {
      "id": "SCHEDULE",
      "label": "Posting Schedule",
      "description": "When to post (cron expression)",
      "value_type": "cron",
      "required": true,
      "default_value": "0 9 * * *",
      "example": "0 9 * * * (daily at 9 AM)"
    },
    {
      "id": "MESSAGE",
      "label": "Status Message",
      "description": "The message to post",
      "value_type": "text",
      "required": true,
      "validation_pattern": "^.{1,300}$",
      "example": "Good morning! Today's going to be a great day!"
    }
  ],
  "tags": ["social", "scheduled", "daily"],
  "is_public": true
}
```

### Placeholder Types

| Type | Description | Validation |
|------|-------------|------------|
| `did` | Decentralized Identifier | Must start with `did:` |
| `at_uri` | AT Protocol URI | Must start with `at://` |
| `url` | Web URL | Must start with `http://` or `https://` |
| `collection` | AT Protocol collection | Must contain `.`, no `/` |
| `cron` | Cron expression | 5-field format or special strings |
| `text` | Any text | No validation |
| `number` | Numeric value | Must parse as number |
| `boolean` | Boolean value | Must be exactly `true` or `false` |
| `json` | JSON value | Must be valid JSON |

### Instantiating from Prototype

Provide values for placeholders:
```json
{
  "USER_DID": "did:plc:alice",
  "SCHEDULE": "0 10 * * *",
  "MESSAGE": "Starting my day with coffee and code!"
}
```

The system replaces all `$[PLACEHOLDER_NAME]` occurrences with actual values.

## Best Practices

### Entry Node Configuration
- Use specific collections when possible to reduce processing overhead
- Use DID filters for user-specific automation
- Combine multiple filters for precise targeting

### Condition Expressions
- Use DataLogic expressions with `{"val": [...]}` to access nested data
- Test conditions thoroughly with sample data
- Keep expressions simple and readable
- Use logical operators like `{"and": [...]}`, `{"or": [...]}` for complex conditions
- Remember conditions must evaluate to boolean values

### Transform Templates
- Use DataLogic expressions for dynamic content generation
- Include proper `$type` and `createdAt` fields for AT Protocol records
- Wrap record data in a `"record"` field for publish_record nodes
- Use `{"now": []}` and `{"datetime": [...]}` for timestamps
- Test templates with actual event data to verify correct field access

### Security Considerations
- Never hardcode sensitive information in blueprints
- Use environment variables for API keys and secrets
- Validate input data in condition nodes
- Be mindful of rate limits when publishing records

### Performance Optimization
- Place condition nodes early to filter events before expensive operations
- Use specific entry node filters to reduce unnecessary processing
- Avoid complex transformations in high-volume scenarios
- Consider using debug_action nodes sparingly in production

## Testing and Debugging

### Using Debug Actions
1. Place debug_action nodes at various pipeline stages
2. Monitor logs for pretty-printed JSON output
3. Verify data structure and transformations
4. Remove debug nodes before production deployment

### Testing Strategies
1. **Unit Testing**: Test individual nodes with known data
2. **Integration Testing**: Test complete blueprints with sample events
3. **Load Testing**: Verify performance with high event volumes
4. **Edge Case Testing**: Test with malformed or unexpected data

### Common Issues

**Blueprint Not Triggering:**
- Check entry node configuration (collections, DIDs, cron expressions)
- Verify the entry node payload condition is met
- Check if blueprint is enabled and properly saved

**Data Access Errors:**
- Use debug_action to inspect available data at each stage
- Verify field paths in `{"val": [...]}` expressions
- Check for typos in field names

**Transform Failures:**
- Validate DataLogic syntax
- Ensure all referenced fields exist
- Test with actual event data structure

**Performance Issues:**
- Reduce complexity of DataLogic expressions
- Move expensive operations to later nodes
- Use appropriate entry node filters

### Monitoring Blueprint Performance
- Monitor blueprint evaluation success/failure rates
- Track execution time for complex blueprints
- Watch for memory usage with large data transformations
- Set up alerts for blueprint evaluation errors

This guide provides the foundation for creating effective blueprints. For additional details on specific features, refer to the configuration guide and API documentation.