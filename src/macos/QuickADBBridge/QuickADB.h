/* Copyright (C) 2026 QuickADB contributors. SPDX-License-Identifier: GPL-3.0-only */
#ifndef QUICKADB_H
#define QUICKADB_H

typedef struct QuickADBBackend QuickADBBackend;

/* Calls for a handle are serialized by the macOS main thread. */
QuickADBBackend * _Nullable quickadb_open(
    const char * _Nonnull directory, char * _Nullable * _Nonnull error);
char * _Nonnull quickadb_snapshot(const QuickADBBackend * _Nonnull app);
char * _Nullable quickadb_command(QuickADBBackend * _Nonnull app, const char * _Nonnull command);
void quickadb_close(QuickADBBackend * _Nonnull app);
void quickadb_string_free(char * _Nonnull text);

#endif
