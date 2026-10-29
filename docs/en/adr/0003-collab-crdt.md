# ADR-0003: Collaboration uses CRDTs, strictly separated from audio PCM

**English** | [简体中文](../../adr/0003-collab-crdt.md)

**Status**: Proposed (ADR only, not implemented)

**Date**: 2026-10-27

## Context

Multi-user collaborative editing needs data synchronisation without affecting the real-time audio path.

## Decision

Answers to three questions:

1. **What is shared**: presets, graph structure, playlists. Not device selection, exclusive resources or live playback state.
2. **Who owns the output**: each participant's local engine outputs independently; collaboration syncs parameters and
   structure only, never PCM, and doesn't require everyone to hear the same mix. "Shared listening" is a separate
   feature to be evaluated on its own.
3. **Conflict rules**: parameters merge via CRDT; device selection and play / stop are decided by a single session host and stay out of the CRDT.

## Constraints

- Automerge only produces **validated `EngineCommand`s** in the bridge layer; PCM never enters the CRDT.
- CRDT documents and audio PCM never meet.

## Consequences

- Phase one keeps only the `collab/` directory and this ADR; no sync logic is written.
