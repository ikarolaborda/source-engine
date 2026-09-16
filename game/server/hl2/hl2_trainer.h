//========= Half-Life 2 trainer ============//
//
// Purpose: in-game cheat toggles (god mode, infinite ammo, infinite suit
//          auxiliary power, multi-jump). None of them require sv_cheats.
//
//=============================================================================//
#ifndef HL2_TRAINER_H
#define HL2_TRAINER_H
#ifdef _WIN32
#pragma once
#endif

#include "convar.h"

class CHL2_Player;

extern ConVar trainer_god;
extern ConVar trainer_infinite_ammo;
extern ConVar trainer_infinite_aux;
extern ConVar trainer_multijump;	// defined in game/shared/gamemovement.cpp (replicated to the client)

// Called from CHL2_Player::PostThink() once per frame.
void Trainer_PlayerPostThink( CHL2_Player *pPlayer );

#endif // HL2_TRAINER_H
