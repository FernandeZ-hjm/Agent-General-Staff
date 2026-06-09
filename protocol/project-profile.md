# Project Profile Protocol

`project-profile.md` defines the public schema for a project-specific AGS
profile. It is intentionally safe to ship as an empty protocol file in the AGS
public suite: real project identity is created by the user when they integrate
AGS into their own repository.

## Purpose

The project profile records durable facts that help agents understand what a
repository is for before they form a solution or produce a task card.

## User-Populated Fields

Integrated projects may maintain these sections:

- project name and short description;
- owner or responsible team;
- repository role and release channel;
- allowed execution surfaces;
- protected paths and non-goals;
- standard verification commands;
- project-specific stop conditions.

## Public Suite Default

The AGS public suite must not include a real user profile, private project
identity, private repository paths, customer data, secrets, or local machine
state. Public distributions may include this protocol file and blank templates
only.

## Related Templates

Use `templates/memory/context-capsule.md` and `templates/memory/task-memory.md`
as starting points when bootstrapping a project-specific memory capsule.
