# Tasks API

**Bridges static code → production API**

Based on [`examples/tasks/`](../tasks/) - same domain model, now with HTTP + Database.

## What This Shows

- ✅ Converting static structs to DB models with decorators
- ✅ Using `app.crud()` for standard endpoints (80% of work)
- ✅ Writing custom handlers when CRUD isn't enough

## Run

```bash
# 1. Setup database
DATABASE_URL=postgres://user:pass@localhost:5432/tasks_db

# 2. Run
doo run main.doo
```

## Endpoints

| Endpoint | Source |
|----------|--------|
| CRUD `/tasks` | `app.crud()` |
| GET `/tasks/done` | Custom handler |
| GET `/tasks/urgent` | Custom handler |
| GET `/tasks/stats` | Custom handler |

---

**For pure Doo learning:** See [`examples/tasks/`](../tasks/)  
**For production templates:** Run `doo init todo`
