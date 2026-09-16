//========= Half-Life 2 trainer ============//
//
// Purpose: in-game cheat toggles compiled into the game DLL:
//   trainer_god            player takes no damage
//   trainer_infinite_ammo  clips and reserve ammo of carried weapons stay full
//   trainer_infinite_aux   suit auxiliary power (sprint/flashlight/oxygen) never drains
//   trainer_multijump      extra jumps while airborne (shared, see gamemovement.cpp)
//   trainer_onehitkill     anything the player damages dies (see baseentity.cpp)
// Each has a *_toggle console command that also prints its new state on the
// HUD, so they can be bound to keys (see cfg/trainer.cfg).
//
//=============================================================================//
#include "cbase.h"
#include "hl2_trainer.h"
#include "hl2_player.h"
#include "basehlcombatweapon_shared.h"
#include "ammodef.h"

// memdbgon must be the last include file in a .cpp file!!!
#include "tier0/memdbgon.h"

ConVar trainer_god( "trainer_god", "0", FCVAR_NONE, "Trainer: 1 = player takes no damage" );
ConVar trainer_infinite_ammo( "trainer_infinite_ammo", "0", FCVAR_NONE, "Trainer: 1 = clips and reserve ammo stay full" );
ConVar trainer_infinite_aux( "trainer_infinite_aux", "0", FCVAR_NONE, "Trainer: 1 = suit auxiliary power (sprint, flashlight, oxygen) never drains" );

//-----------------------------------------------------------------------------
static void Trainer_Announce( CBasePlayer *pPlayer, const char *pszText )
{
	Msg( "[trainer] %s\n", pszText );
	if ( pPlayer )
	{
		ClientPrint( pPlayer, HUD_PRINTCENTER, pszText );
	}
}

static void Trainer_Toggle( ConVar &cvar, const char *pszName )
{
	bool bOn = !cvar.GetBool();
	cvar.SetValue( bOn ? 1 : 0 );

	char szText[64];
	Q_snprintf( szText, sizeof( szText ), "%s: %s", pszName, bOn ? "ON" : "OFF" );
	Trainer_Announce( UTIL_GetCommandClient(), szText );
}

CON_COMMAND( trainer_god_toggle, "Trainer: toggle god mode" )
{
	Trainer_Toggle( trainer_god, "GOD MODE" );
}

CON_COMMAND( trainer_ammo_toggle, "Trainer: toggle infinite ammo" )
{
	Trainer_Toggle( trainer_infinite_ammo, "INFINITE AMMO" );
}

CON_COMMAND( trainer_aux_toggle, "Trainer: toggle infinite suit auxiliary power (sprint)" )
{
	Trainer_Toggle( trainer_infinite_aux, "INFINITE AUX POWER" );
}

CON_COMMAND( trainer_jump_toggle, "Trainer: toggle multi-jump (unlimited air jumps; set trainer_multijump N for a limit)" )
{
	bool bOn = ( trainer_multijump.GetInt() == 0 );
	trainer_multijump.SetValue( bOn ? -1 : 0 );
	Trainer_Announce( UTIL_GetCommandClient(), bOn ? "MULTI-JUMP: ON" : "MULTI-JUMP: OFF" );
}

CON_COMMAND( trainer_kill_toggle, "Trainer: toggle one-hit kill" )
{
	Trainer_Toggle( trainer_onehitkill, "ONE-HIT KILL" );
}

CON_COMMAND( trainer_status, "Trainer: show the state of every trainer option" )
{
	int nJumps = trainer_multijump.GetInt();
	char szText[224];
	Q_snprintf( szText, sizeof( szText ), "God %s | Ammo %s | Aux %s | Multi-jump %s | 1-hit kill %s",
		trainer_god.GetBool() ? "ON" : "off",
		trainer_infinite_ammo.GetBool() ? "ON" : "off",
		trainer_infinite_aux.GetBool() ? "ON" : "off",
		nJumps == 0 ? "off" : ( nJumps < 0 ? "ON (unlimited)" : "ON" ),
		trainer_onehitkill.GetBool() ? "ON" : "off" );
	Trainer_Announce( UTIL_GetCommandClient(), szText );
}

//-----------------------------------------------------------------------------
// Keep the clip and reserve ammo of every carried weapon topped up.
//-----------------------------------------------------------------------------
static void Trainer_RefillAmmo( CHL2_Player *pPlayer )
{
	CAmmoDef *pAmmoDef = GetAmmoDef();

	for ( int i = 0; i < pPlayer->WeaponCount(); i++ )
	{
		CBaseCombatWeapon *pWeapon = pPlayer->GetWeapon( i );
		if ( !pWeapon )
			continue;

		if ( pWeapon->UsesClipsForAmmo1() && pWeapon->m_iClip1 < pWeapon->GetMaxClip1() )
			pWeapon->m_iClip1 = pWeapon->GetMaxClip1();

		if ( pWeapon->UsesClipsForAmmo2() && pWeapon->m_iClip2 < pWeapon->GetMaxClip2() )
			pWeapon->m_iClip2 = pWeapon->GetMaxClip2();

		int nAmmoTypes[2] = { pWeapon->GetPrimaryAmmoType(), pWeapon->GetSecondaryAmmoType() };
		for ( int j = 0; j < 2; j++ )
		{
			int nType = nAmmoTypes[j];
			if ( nType < 0 )
				continue;

			int nMax = pAmmoDef->MaxCarry( nType );
			if ( nMax > 0 && pPlayer->GetAmmoCount( nType ) < nMax )
				pPlayer->SetAmmoCount( nMax, nType );
		}
	}
}

//-----------------------------------------------------------------------------
void Trainer_PlayerPostThink( CHL2_Player *pPlayer )
{
	if ( !pPlayer || !pPlayer->IsAlive() )
		return;

	if ( trainer_infinite_ammo.GetBool() )
	{
		Trainer_RefillAmmo( pPlayer );
	}

	if ( trainer_infinite_aux.GetBool() )
	{
		pPlayer->SuitPower_SetCharge( 100.0f );
	}
}
