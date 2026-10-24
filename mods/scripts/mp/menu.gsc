// bo2mp default mod menu (OUR CODE). Edit freely: the game reads this file
// every match, no rebuild needed. A mistake here is named in the game's log
// ("bo2mp mods: scripts/mp/menu left out: ...") and the match runs without it.
//
// Open:   Aim + Knife (hold aim, tap knife)
// Move:   Aim = up, Fire = down
// Pick:   Use (F / X)
// Back:   Knife (closes the menu from the main page)
//
// Add an option: one line in build_menus(), e.g.
//     self add_option( "main", "My thing", ::my_thing );
// and write my_thing() below; `self` is the player who picked it.
// A submenu: self add_menu( "name", "Title", "main" ); then
//     self add_option( "main", "Open it", ::open_menu, "name" );

init()
{
    level.mm_fx = loadfx( "explosions/fx_grenadeexp_default" );
    level thread menu_players();
}

// Every human player gets the menu once (bots never).
menu_players()
{
    level endon( "game_ended" );
    for ( ;; )
    {
        players = getplayers();
        for ( i = 0; i < players.size; i++ )
        {
            p = players[i];
            if ( isdefined( p.mm ) )
                continue;
            if ( p istestclient() )
                continue;
            p.mm = spawnstruct();
            p thread menu_player();
        }
        wait 0.5;
    }
}

menu_player()
{
    self endon( "disconnect" );
    self.mm.toggles = [];
    self build_menus();
    self.mm.open = false;
    self.mm.cur = "main";
    self.mm.pos = 0;
    self.mm.elems = [];
    self thread menu_hint();
    self thread menu_buttons();
    self thread menu_respawn();
}

menu_hint()
{
    self endon( "disconnect" );
    wait 3;
    self iprintln( "^3Aim + Knife^7: mod menu" );
}

// ---------------------------------------------------------------------------
// The menus. Add, remove or reorder lines here.
// ---------------------------------------------------------------------------

build_menus()
{
    self.mm.menus = [];
    self.mm.names = [];
    self.mm.streaks = [];

    self add_menu( "main", "bo2mp Mod Menu", undefined );
    self add_option( "main", "Main Mods", ::open_menu, "mods" );
    self add_option( "main", "Weapons", ::open_menu, "weapons" );
    self add_option( "main", "Scorestreaks", ::open_menu, "streaks" );
    self add_option( "main", "Trickshots", ::open_menu, "trick" );
    self add_option( "main", "Game", ::open_menu, "game" );
    self add_option( "main", "Bots", ::open_menu, "bots" );
    self add_option( "main", "Fun", ::open_menu, "fun" );
    self add_option( "main", "Map Test", ::open_menu, "maptest" );

    self add_menu( "mods", "Main Mods", "main" );
    self add_option( "mods", "God Mode", ::toggle_god );
    self add_option( "mods", "Infinite Ammo", ::toggle_ammo );
    self add_option( "mods", "UFO Mode", ::toggle_ufo );
    self add_option( "mods", "Third Person", ::toggle_third );
    self add_option( "mods", "Speed x2", ::toggle_speed );
    self add_option( "mods", "Teleport to Crosshair", ::teleport_crosshair );
    self add_option( "mods", "Save Position", ::save_position );
    self add_option( "mods", "Load Position", ::load_position );
    self add_option( "mods", "Max Ammo", ::max_ammo );
    self add_option( "mods", "Invisible", ::toggle_invisible );
    self add_option( "mods", "Super Jump", ::toggle_super_jump );
    self add_option( "mods", "Low Gravity", ::toggle_low_gravity );
    self add_option( "mods", "No Recoil", ::toggle_no_recoil );
    self add_option( "mods", "Rapid Fire", ::toggle_rapid_fire );
    self add_option( "mods", "Explosive Bullets", ::toggle_explosive );
    self add_option( "mods", "Infinite Equipment", ::toggle_equipment );

    self add_menu( "trick", "Trickshots", "main" );
    self add_option( "trick", "Spawn Platform", ::spawn_platform );
    self add_option( "trick", "Remove Platforms", ::remove_platforms );
    self add_option( "trick", "Canswap (drop a random gun)", ::canswap );
    self add_option( "trick", "Fast Last", ::fast_last );

    self add_menu( "game", "Game", "main" );
    self add_option( "game", "Pause the Clock", ::toggle_pause_clock );
    self add_option( "game", "Unlimited Score", ::toggle_unlimited_score );
    self add_option( "game", "Slow Motion", ::toggle_slow_motion );
    self add_option( "game", "Change Map", ::open_menu, "maps" );
    self add_option( "game", "Change Mode", ::open_menu, "modes" );
    self add_list( "maps", "Change Map", "game", ::change_map, "mp_la Aftermath;mp_dockside Cargo;mp_carrier Carrier;mp_drone Drone;mp_express Express;mp_hijacked Hijacked;mp_meltdown Meltdown;mp_overflow Overflow;mp_nightclub Plaza;mp_raid Raid;mp_slums Slums;mp_village Standoff;mp_turbine Turbine;mp_socotra Yemen;mp_nuketown_2020 Nuketown 2025;mp_downhill Downhill;mp_mirage Mirage;mp_hydro Hydro;mp_skate Grind;mp_concert Encore;mp_magma Magma;mp_vertigo Vertigo;mp_studio Studio;mp_uplink Uplink;mp_bridge Detour;mp_castaway Cove;mp_paintball Rush;mp_dig Dig;mp_frostbite Frost;mp_pod Pod;mp_takeoff Takeoff" );
    self add_list( "modes", "Change Mode", "game", ::change_mode, "tdm Team Deathmatch;dm Free-for-All;dom Domination;conf Kill Confirmed" );

    self add_menu( "weapons", "Weapons", "main" );
    self add_option( "weapons", "Assault Rifles", ::open_menu, "w_ar" );
    self add_option( "weapons", "SMGs", ::open_menu, "w_smg" );
    self add_option( "weapons", "Shotguns + LMGs", ::open_menu, "w_heavy" );
    self add_option( "weapons", "Snipers", ::open_menu, "w_sniper" );
    self add_option( "weapons", "Pistols", ::open_menu, "w_pistol" );
    self add_option( "weapons", "Launchers + Specials", ::open_menu, "w_special" );
    self add_option( "weapons", "Zombies Wonder Weapons", ::open_menu, "w_zm" );
    self add_option( "weapons", "Zombies Specials", ::open_menu, "w_zm2" );
    self add_option( "weapons", "Pack-a-Punch Current Weapon", ::pap_weapon );
    self add_option( "weapons", "Take Current Weapon", ::take_weapon );

    self add_weapons( "w_ar", "Assault Rifles", "tar21_mp MTAR;type95_mp Type 25;sig556_mp SWAT-556;sa58_mp FAL OSW;hk416_mp M27;scar_mp SCAR-H;saritch_mp SMR;xm8_mp M8A1;an94_mp AN-94" );
    self add_weapons( "w_smg", "SMGs", "mp7_mp MP7;pdw57_mp PDW-57;vector_mp Vector K10;insas_mp MSMC;qcw05_mp Chicom CQB;evoskorpion_mp Skorpion EVO;peacekeeper_mp Peacekeeper" );
    self add_weapons( "w_heavy", "Shotguns + LMGs", "870mcs_mp R870 MCS;saiga12_mp S12;ksg_mp KSG;srm1216_mp M1216;mk48_mp Mk 48;qbb95_mp QBB LSW;lsat_mp LSAT;hamr_mp HAMR" );
    self add_weapons( "w_sniper", "Snipers", "svu_mp SVU-AS;dsr50_mp DSR 50;ballista_mp Ballista;as50_mp XPR-50" );
    self add_weapons( "w_pistol", "Pistols", "fiveseven_mp Five-seven;fnp45_mp Tac-45;beretta93r_mp B23R;judge_mp Executioner;kard_mp KAP-40" );
    self add_weapons( "w_special", "Launchers + Specials", "smaw_mp SMAW;fhj18_mp FHJ-18 AA;usrpg_mp RPG;riotshield_mp Assault Shield;crossbow_mp Crossbow;knife_ballistic_mp Ballistic Knife;minigun_mp Death Machine;m32_mp War Machine;defaultweapon_mp Default Weapon" );

    self add_weapons( "w_zm", "Zombies Wonder Weapons", "ray_gun_zm Ray Gun;raygun_mark2_zm Ray Gun Mark II;jetgun_zm Jet Gun;slipgun_zm Sliquifier;slowgun_zm Paralyzer;blundergat_zm Blundergat;blundersplat_zm Acidgat;staff_fire_zm Fire Staff;staff_air_zm Wind Staff;staff_lightning_zm Lightning Staff;staff_water_zm Ice Staff;physcannon_zm Gravity Gun;physcannon_upgraded_zm Super Gravity Gun;mcbow_zm Bow;mcbow_enchanted_zm Enchanted Bow;mcstick_zm Enchanted Stick;portalgun_zm Portal Gun;portalgun_upgraded_zm Super Portal Gun" );
    self add_weapons( "w_zm2", "Zombies Specials", "knife_ballistic_zm Ballistic Knife (Zombies);cymbal_monkey_zm Monkey Bomb;emp_grenade_zm EMP Grenade;time_bomb_zm Time Bomb;beacon_zm G-Strike;bouncing_tomahawk_zm Hell's Retriever;upgraded_tomahawk_zm Hell's Redeemer;riotshield_zm Zombie Shield;bowie_knife_zm Bowie Knife;tazer_knuckles_zm Galvaknuckles" );

    self add_menu( "streaks", "Scorestreaks", "main" );
    self add_streaks( "radar_mp UAV;rcbomb_mp RC-XD;counteruav_mp Counter-UAV;missile_drone_mp Hunter Killer;supply_drop_mp Care Package;microwaveturret_mp Guardian;remote_missile_mp Hellstorm;planemortar_mp Lightning Strike;autoturret_mp Sentry Gun;minigun_mp Death Machine;m32_mp War Machine;qrdrone_mp Dragonfire;ai_tank_drop_mp A.G.R;helicopter_comlink_mp Stealth Chopper;radardirection_mp Orbital VSAT;helicopter_guard_mp Escort Drone;emp_mp EMP;remote_mortar_mp Lodestar;straferun_mp Warthog;helicopter_player_gunner_mp VTOL Warship;dogs_mp K9 Unit;missile_swarm_mp Swarm" );

    self add_menu( "bots", "Bots", "main" );
    self add_option( "bots", "Add Enemy Bot", ::add_bot, "enemy" );
    self add_option( "bots", "Add Friendly Bot", ::add_bot, "friendly" );
    self add_option( "bots", "Freeze All Bots", ::toggle_freeze_bots );
    self add_option( "bots", "Bring Bots To Me", ::bring_bots );
    self add_option( "bots", "Bots Don't Shoot", ::toggle_bots_no_shoot );
    self add_option( "bots", "Kill All Bots", ::kill_bots );
    self add_option( "bots", "Kick All Bots", ::kick_bots );
    self add_option( "bots", "Enemy Calls UAV", ::enemy_streak, "radar_mp" );
    self add_option( "bots", "Enemy Calls Counter-UAV", ::enemy_streak, "counteruav_mp" );

    self add_menu( "fun", "Fun", "main" );
    self add_option( "fun", "Kill All Enemies", ::kill_enemies );
    self add_option( "fun", "Fast Restart", ::fast_restart );
    self add_option( "fun", "Suicide", ::suicide_me );

    // Map Test: check a map's breakable and moving things by hand.
    self add_menu( "maptest", "Map Test", "main" );
    self add_option( "maptest", "Go To...", ::open_menu, "mt_spots" );
    self add_option( "maptest", "What Am I Looking At", ::mt_look );
    self add_option( "maptest", "Test Guns", ::open_menu, "mt_guns" );
    self add_option( "maptest", "Car Blast at Crosshair", ::mt_blast );
    self add_option( "maptest", "Rocket at Crosshair", ::mt_rocket );
    self add_option( "maptest", "Next Mannequin Head", ::mt_next_head );
    self add_option( "maptest", "Mannequin Egg Status", ::mt_egg );
    self add_option( "maptest", "End Match Now", ::mt_end );
    self add_list( "mt_spots", "Go To...", "maptest", ::mt_go, mt_spot_list() );
    self add_weapons( "mt_guns", "Test Guns", "an94_mp AN-94 (bullets);mk48_mp Mk 48 (bullets);smaw_mp SMAW (rocket);crossbow_mp Crossbow (sticky bolt);sticky_grenade_mp Semtex;satchel_charge_mp C4;frag_grenade_mp Frag;knife_ballistic_mp Ballistic Knife" );
    self.mm.menus["mt_guns"].parent = "maptest";
}

add_menu( name, title, parent )
{
    m = spawnstruct();
    m.title = title;
    m.parent = parent;
    m.labels = [];
    m.funcs = [];
    m.args = [];
    self.mm.menus[name] = m;
}

add_option( menu, label, func, arg )
{
    m = self.mm.menus[menu];
    i = m.labels.size;
    m.labels[i] = label;
    m.funcs[i] = func;
    m.args[i] = arg;
}

// "weapon_mp Name;weapon_mp Name": one option per weapon.
add_weapons( name, title, list )
{
    self add_menu( name, title, "weapons" );
    items = strtok( list, ";" );
    for ( i = 0; i < items.size; i++ )
    {
        w = item_key( items[i] );
        self.mm.names[w] = item_label( items[i] );
        self add_option( name, self.mm.names[w], ::give_gun, w );
    }
}

// "key Name;key Name": one option per item, each calling func( key ).
add_list( name, title, parent, func, list )
{
    self add_menu( name, title, parent );
    items = strtok( list, ";" );
    for ( i = 0; i < items.size; i++ )
        self add_option( name, item_label( items[i] ), func, item_key( items[i] ) );
}

add_streaks( list )
{
    items = strtok( list, ";" );
    for ( i = 0; i < items.size; i++ )
    {
        k = item_key( items[i] );
        self.mm.streaks[k] = item_label( items[i] );
        self add_option( "streaks", self.mm.streaks[k], ::give_streak, k );
    }
}

item_key( item )
{
    return strtok( item, " " )[0];
}

item_label( item )
{
    words = strtok( item, " " );
    s = "";
    for ( i = 1; i < words.size; i++ )
    {
        if ( i > 1 )
            s = s + " ";
        s = s + words[i];
    }
    return s;
}

// ---------------------------------------------------------------------------
// Buttons and drawing
// ---------------------------------------------------------------------------

// A press counts once the button was let go for two checks in a row (the
// engine can read a held button as up for one frame). Holding Aim or Fire
// keeps scrolling.
menu_buttons()
{
    self endon( "disconnect" );
    self.mm.up = [];
    self.mm.down = [];
    names = strtok( "ads attack use melee", " " );
    for ( i = 0; i < names.size; i++ )
    {
        self.mm.up[names[i]] = 0;
        self.mm.down[names[i]] = 0;
    }
    for ( ;; )
    {
        ads = self button( "ads", self adsbuttonpressed() );
        attack = self button( "attack", self attackbuttonpressed() );
        use = self button( "use", self usebuttonpressed() );
        melee = self button( "melee", self meleebuttonpressed() );
        if ( !self.mm.open )
        {
            if ( melee && self adsbuttonpressed() && isalive( self ) )
                self menu_open();
        }
        else if ( !isalive( self ) )
            self menu_close();
        else if ( attack || self repeat( "attack" ) )
            self menu_move( 1 );
        else if ( ads || self repeat( "ads" ) )
            self menu_move( -1 );
        else if ( use )
            self menu_pick();
        else if ( melee )
            self menu_back();
        wait 0.05;
    }
}

// True on a fresh press of this button.
button( name, down )
{
    if ( !down )
    {
        self.mm.up[name] = self.mm.up[name] + 1;
        if ( self.mm.up[name] >= 2 )
            self.mm.down[name] = 0;
        return false;
    }
    self.mm.up[name] = 0;
    self.mm.down[name] = self.mm.down[name] + 1;
    return self.mm.down[name] == 1;
}

// Held for 0.4 s: one more step every 0.1 s.
repeat( name )
{
    n = self.mm.down[name];
    return n >= 8 && n % 2 == 0;
}

menu_open()
{
    self.mm.open = true;
    // Aim is still held from opening: it moves nothing until let go.
    self.mm.down["ads"] = -100000;
    self.mm.cur = "main";
    self.mm.pos = 0;
    self disableweapons();
    self draw_menu();
}

menu_close()
{
    if ( !self.mm.open )
        return;
    self.mm.open = false;
    self clear_menu();
    self thread menu_weapons_back();
}

// The gun comes back once Knife is let go: closing with Knife never knifes
// whoever stands in front of him.
menu_weapons_back()
{
    self endon( "disconnect" );
    while ( self meleebuttonpressed() )
        wait 0.05;
    if ( !self.mm.open )
        self enableweapons();
}

menu_move( step )
{
    n = self.mm.menus[self.mm.cur].labels.size;
    self.mm.pos = self.mm.pos + step;
    if ( self.mm.pos < 0 )
        self.mm.pos = n - 1;
    if ( self.mm.pos >= n )
        self.mm.pos = 0;
    self draw_menu();
}

menu_pick()
{
    m = self.mm.menus[self.mm.cur];
    i = self.mm.pos;
    if ( isdefined( m.args[i] ) )
        self thread [[ m.funcs[i] ]]( m.args[i] );
    else
        self thread [[ m.funcs[i] ]]();
}

menu_back()
{
    parent = self.mm.menus[self.mm.cur].parent;
    if ( !isdefined( parent ) )
    {
        self menu_close();
        return;
    }
    self open_menu( parent );
}

open_menu( name )
{
    self.mm.cur = name;
    self.mm.pos = 0;
    self draw_menu();
}

clear_menu()
{
    for ( i = 0; i < self.mm.elems.size; i++ )
        self.mm.elems[i] destroy();
    self.mm.elems = [];
}

menu_elem( x, y, sort )
{
    e = newclienthudelem( self );
    e.alignx = "left";
    e.aligny = "top";
    e.horzalign = "left";
    e.vertalign = "top";
    e.x = x;
    e.y = y;
    e.sort = sort;
    e.alpha = 1;
    e.hidewheninmenu = 1;
    self.mm.elems[self.mm.elems.size] = e;
    return e;
}

menu_text( text, x, y, scale, color, sort )
{
    e = self menu_elem( x, y, sort );
    e.font = "default";
    e.fontscale = scale;
    e.color = color;
    e settext( text );
    return e;
}

menu_box( x, y, w, h, color, alpha, sort )
{
    e = self menu_elem( x, y, sort );
    e.color = color;
    e setshader( "white", w, h );
    e.alpha = alpha;
    return e;
}

// The whole page, drawn again on every change (a few elements, cheap).
// Long pages show ROWS options at a time and scroll with the cursor.
draw_menu()
{
    self clear_menu();
    m = self.mm.menus[self.mm.cur];
    rows = 10;
    first = 0;
    if ( self.mm.pos >= rows )
        first = self.mm.pos - rows + 1;
    shown = m.labels.size - first;
    if ( shown > rows )
        shown = rows;
    x = 20;
    y = 128;
    row = 18;
    w = 230;
    h = 34 + shown * row + 22;
    self menu_box( x, y, w, h, ( 0, 0, 0 ), 0.7, 1 );
    self menu_box( x, y, w, 26, ( 1, 0.5, 0 ), 0.9, 2 );
    self menu_box( x, y + 32 + ( self.mm.pos - first ) * row, w, row, ( 1, 0.5, 0 ), 0.45, 2 );
    title = m.title;
    if ( m.labels.size > rows )
        title = title + "  " + ( self.mm.pos + 1 ) + "/" + m.labels.size;
    self menu_text( title, x + 8, y + 4, 1.5, ( 1, 1, 1 ), 3 );
    for ( i = 0; i < shown; i++ )
    {
        k = first + i;
        color = ( 0.85, 0.85, 0.85 );
        if ( k == self.mm.pos )
            color = ( 1, 1, 1 );
        self menu_text( m.labels[k] + self state_tag( m.labels[k] ), x + 10, y + 33 + i * row, 1.2, color, 3 );
    }
    self menu_text( "Aim/Fire move   Use pick   Knife back", x + 8, y + h - 18, 0.9, ( 0.7, 0.7, 0.7 ), 3 );
}

// " [ON]" after a toggle that is on (toggles are kept by their label).
state_tag( label )
{
    if ( isdefined( self.mm.toggles[label] ) && self.mm.toggles[label] )
        return " ^2[ON]";
    return "";
}

// ---------------------------------------------------------------------------
// The options
// ---------------------------------------------------------------------------

say( text )
{
    self iprintln( text );
    if ( self.mm.open )
        self draw_menu();
}

flip( name )
{
    on = !( isdefined( self.mm.toggles[name] ) && self.mm.toggles[name] );
    self.mm.toggles[name] = on;
    return on;
}

toggle_god()
{
    if ( self flip( "God Mode" ) )
    {
        self enableinvulnerability();
        self say( "God Mode ^2ON" );
    }
    else
    {
        self disableinvulnerability();
        self say( "God Mode ^1OFF" );
    }
}

toggle_ammo()
{
    self notify( "mm_ammo_off" );
    if ( self flip( "Infinite Ammo" ) )
    {
        self thread ammo_loop();
        self say( "Infinite Ammo ^2ON" );
    }
    else
        self say( "Infinite Ammo ^1OFF" );
}

ammo_loop()
{
    self endon( "disconnect" );
    self endon( "mm_ammo_off" );
    for ( ;; )
    {
        w = self getcurrentweapon();
        if ( isdefined( w ) && w != "none" )
        {
            self setweaponammoclip( w, 999 );
            self givemaxammo( w );
        }
        wait 0.1;
    }
}

toggle_ufo()
{
    self notify( "mm_ufo_off" );
    if ( self flip( "UFO Mode" ) )
    {
        self setufomode( 1 );
        self thread ufo_watch();
        self say( "UFO ^2ON^7 (look and walk to fly, Jump = up, Crouch = down, Sprint = fast)" );
    }
    else
    {
        self setufomode( 0 );
        self say( "UFO ^1OFF" );
    }
}

// Dying ends UFO (the engine drops it); the toggle follows.
ufo_watch()
{
    self endon( "disconnect" );
    self endon( "mm_ufo_off" );
    self waittill( "death" );
    self.mm.toggles["UFO Mode"] = 0;
}

waittill_any_mm( a, b, c )
{
    self endon( a );
    self endon( b );
    self endon( c );
    self waittill( "mm_never" );
}

toggle_third()
{
    on = self flip( "Third Person" );
    self setclientthirdperson( on );
    if ( on )
        self say( "Third Person ^2ON" );
    else
        self say( "Third Person ^1OFF" );
}

toggle_speed()
{
    if ( self flip( "Speed x2" ) )
    {
        self setmovespeedscale( 2 );
        self say( "Speed x2 ^2ON" );
    }
    else
    {
        self setmovespeedscale( 1 );
        self say( "Speed x2 ^1OFF" );
    }
}

teleport_crosshair()
{
    eye = self geteye();
    fwd = anglestoforward( self getplayerangles() );
    trace = bullettrace( eye, eye + vectorscale( fwd, 100000 ), false, self );
    if ( trace["fraction"] >= 1 )
    {
        self say( "Nothing there to teleport to" );
        return;
    }
    // A step back off the wall, then onto the floor under that spot.
    spot = trace["position"] - vectorscale( fwd, 32 );
    ground = bullettrace( spot + ( 0, 0, 8 ), spot - ( 0, 0, 2000 ), false, self );
    if ( ground["fraction"] < 1 )
        spot = ground["position"];
    self setorigin( spot );
    self setvelocity( ( 0, 0, 0 ) );
    self say( "Teleported" );
}

save_position()
{
    self.mm.saved_origin = self.origin;
    self.mm.saved_angles = self getplayerangles();
    self say( "Position saved" );
}

load_position()
{
    if ( !isdefined( self.mm.saved_origin ) )
    {
        self say( "Save a position first" );
        return;
    }
    self setorigin( self.mm.saved_origin );
    self setplayerangles( self.mm.saved_angles );
    self say( "Position loaded" );
}

max_ammo()
{
    w = self getcurrentweapon();
    self givemaxammo( w );
    self setweaponammoclip( w, 999 );
    self say( "Max ammo" );
}

// Like a Zombies wall buy: with two guns already, the one in your hands
// goes. A grenade or other item swaps for the last one the menu gave, so the
// weapon slots never fill up and every gun comes with its ammo.
give_gun( weapon )
{
    if ( weaponinventorytype( weapon ) == "primary" )
    {
        guns = self getweaponslistprimaries();
        if ( guns.size >= 2 && !self hasweapon( weapon ) )
        {
            old = guns[0];
            current = self getcurrentweapon();
            for ( i = 0; i < guns.size; i++ )
            {
                if ( guns[i] == current )
                    old = current;
            }
            self takeweapon( old );
        }
    }
    else
    {
        if ( isdefined( self.mm.item ) && self.mm.item != weapon )
            self takeweapon( self.mm.item );
        self.mm.item = weapon;
    }
    self giveweapon( weapon );
    self givemaxammo( weapon );
    self switchtoweapon( weapon );
    self say( "Gave " + self.mm.names[weapon] );
}

// Pack-a-Punch: the gun he holds swaps for its Black Ops II Zombies
// upgraded one (the Zombies maps' own, with its camo). Multiplayer guns that
// Zombies never had say so.
pap_weapon()
{
    w = self getcurrentweapon();
    up = pap_name( strtok( w, "+" )[0] );
    if ( !isdefined( up ) )
    {
        self say( "No Pack-a-Punch for this one" );
        return;
    }
    self takeweapon( w );
    self giveweapon( up );
    self givemaxammo( up );
    self switchtoweapon( up );
    self say( "^5Pack-a-Punched!" );
}

pap_name( w )
{
    if ( issubstr( w, "upgraded" ) )
        return undefined;
    // The game names a Zombies gun held in Multiplayer "ray_gun_zm_mp":
    // both endings come off.
    stem = w;
    while ( stem.size > 3 && mm_has( "_mp _zm", getsubstr( stem, stem.size - 3, stem.size ) ) )
        stem = getsubstr( stem, 0, stem.size - 3 );
    if ( stem == "bouncing_tomahawk" )
        return "upgraded_tomahawk_zm";
    if ( stem == "minigun" )
        return "minigun_alcatraz_upgraded_zm";
    if ( stem == "sa58" )
        stem = "fnfal";
    // The bow's Pack-a-Punch is Minecraft's enchanted one.
    if ( stem == "mcbow" )
        return "mcbow_enchanted_zm";
    if ( mm_has( "staff_fire staff_air staff_lightning staff_water", stem ) )
        return stem + "_upgraded_zm";
    paps = "870mcs ak47 ak74u an94 ballista barretm82 beretta93r blundergat blundersplat c96 dsr50 evoskorpion fiveseven fnfal galil hamr jetgun judge kard knife_ballistic ksg lsat m14 m16_gl m1911 m32 mg08 mp40 mp44 mp5k pdw57 physcannon portalgun python qcw05 ray_gun raygun_mark2 rnma rottweil72 rpd saiga12 saritch scar slipgun slowgun srm1216 svu tar21 thompson type95 usrpg uzi xm8";
    if ( mm_has( paps, stem ) )
        return stem + "_upgraded_zm";
    return undefined;
}

mm_has( list, word )
{
    words = strtok( list, " " );
    for ( i = 0; i < words.size; i++ )
    {
        if ( words[i] == word )
            return 1;
    }
    return 0;
}

take_weapon()
{
    w = self getcurrentweapon();
    self takeweapon( w );
    self say( "Took " + w );
}

// Used at once: BO2 fires a streak when you switch to its weapon with the
// score for it, so you get just that score (what you had comes back after)
// and the menu closes and switches you to it.
give_streak( streak )
{
    if ( !isdefined( level.killstreaks[streak] ) )
    {
        self say( self.mm.streaks[streak] + " isn't in this game" );
        return;
    }
    self menu_close();
    weapon = maps\mp\killstreaks\_killstreaks::getkillstreakweapon( streak );
    // BO2's own scorestreak count: one charge, so its waiter calls it in at once
    self maps\mp\killstreaks\_killstreaks::changekillstreakquantity( weapon, 1 );
    self switchtoweapon( weapon );
    self say( "Using " + self.mm.streaks[streak] );
}

add_bot( side )
{
    team = self.pers["team"];
    if ( side == "enemy" )
    {
        if ( team == "allies" )
            team = "axis";
        else
            team = "allies";
    }
    // After Kick All Bots the game stops refilling bots; this one still comes.
    nobots = level.menu_nobots;
    level.menu_nobots = 0;
    maps\mp\bots\_bot::spawn_bot( team );
    level.menu_nobots = nobots;
    self say( "Added a bot to " + team );
}

toggle_freeze_bots()
{
    frozen = self flip( "Freeze All Bots" );
    players = getplayers();
    for ( i = 0; i < players.size; i++ )
    {
        if ( players[i] istestclient() )
            players[i] freezecontrols( frozen );
    }
    if ( frozen )
        self say( "Bots frozen" );
    else
        self say( "Bots unfrozen" );
}

// In rows of 5 in front of you, 64 apart, on the floor, facing you.
bring_bots()
{
    bots = [];
    players = getplayers();
    for ( i = 0; i < players.size; i++ )
    {
        if ( players[i] istestclient() && isalive( players[i] ) )
            bots[bots.size] = players[i];
    }
    if ( bots.size == 0 )
    {
        self say( "No bots to bring" );
        return;
    }
    // In front of you if there's room, else the most open way round.
    rows = int( ( bots.size + 4 ) / 5 );
    need = 150 + ( rows - 1 ) * 80 + 40;
    chest = self.origin + ( 0, 0, 40 );
    look = self getplayerangles()[1];
    yaw = look;
    best = -1;
    turns = [];
    turns[0] = 0; turns[1] = 45; turns[2] = -45; turns[3] = 90; turns[4] = -90; turns[5] = 135; turns[6] = -135; turns[7] = 180;
    for ( t = 0; t < turns.size; t++ )
    {
        dir = anglestoforward( ( 0, look + turns[t], 0 ) );
        clear = bullettrace( chest, chest + vectorscale( dir, need ), false, self )["fraction"] * need;
        if ( clear > best )
        {
            best = clear;
            yaw = look + turns[t];
        }
        if ( clear >= need )
            break;
    }
    fwd = anglestoforward( ( 0, yaw, 0 ) );
    right = anglestoright( ( 0, yaw, 0 ) );
    for ( i = 0; i < bots.size; i++ )
    {
        row = int( i / 5 );
        inrow = bots.size - row * 5;
        if ( inrow > 5 )
            inrow = 5;
        side = ( i % 5 - ( inrow - 1 ) * 0.5 ) * 64;
        spot = chest + vectorscale( fwd, 150 + row * 80 ) + vectorscale( right, side );
        // Never through a wall: stop short of whatever is in the way.
        hit = bullettrace( chest, spot, false, self );
        if ( hit["fraction"] < 1 )
            spot = chest + vectorscale( spot - chest, hit["fraction"] ) - vectorscale( vectornormalize( spot - chest ), 24 );
        ground = bullettrace( spot, spot - ( 0, 0, 500 ), false, bots[i] );
        if ( ground["fraction"] < 1 )
            spot = ground["position"];
        bots[i] setorigin( spot );
        bots[i] setvelocity( ( 0, 0, 0 ) );
        bots[i] setplayerangles( ( 0, yaw + 180, 0 ) );
    }
    self say( "Brought " + bots.size + " bots" );
}

// A living enemy bot uses a scorestreak through BO2's own call (to test
// what an enemy's streak does to you). UAV and Counter-UAV so far.
enemy_streak( streak )
{
    players = getplayers();
    bot = undefined;
    for ( i = 0; i < players.size; i++ )
    {
        p = players[i];
        if ( p == self || !p istestclient() || !isalive( p ) )
            continue;
        if ( level.teambased && p.pers["team"] == self.pers["team"] )
            continue;
        bot = p;
        break;
    }
    if ( !isdefined( bot ) )
    {
        self say( "No living enemy bot (add one in Bots)" );
        return;
    }
    if ( streak == "counteruav_mp" )
        ok = bot maps\mp\killstreaks\_radar::usekillstreakcounteruav( streak );
    else
        ok = bot maps\mp\killstreaks\_radar::usekillstreakradar( streak );
    if ( isdefined( ok ) && ok )
        self say( "Enemy called " + self.mm.streaks[streak] );
    else
        self say( "Enemy can't call " + self.mm.streaks[streak] + " right now" );
}

kill_enemies()
{
    players = getplayers();
    n = 0;
    for ( i = 0; i < players.size; i++ )
    {
        p = players[i];
        if ( p == self || !isalive( p ) )
            continue;
        if ( level.teambased && p.pers["team"] == self.pers["team"] )
            continue;
        p dodamage( p.health + 1000, p.origin, self, self, "none", "MOD_RIFLE_BULLET" );
        n++;
    }
    self say( "Killed " + n );
}

fast_restart()
{
    map_restart( false );
}

suicide_me()
{
    self suicide();
}

// ---------------------------------------------------------------------------
// Player mods (more)
// ---------------------------------------------------------------------------

is_on( name )
{
    return isdefined( self.mm.toggles[name] ) && self.mm.toggles[name];
}

// A new life clears what the engine keeps per life (invisible, recoil,
// rapid fire); the toggles that are on come back with each spawn.
menu_respawn()
{
    self endon( "disconnect" );
    for ( ;; )
    {
        self waittill( "spawned_player" );
        wait 0.1;
        if ( self is_on( "Invisible" ) )
        {
            self.ignoreme = 1;
            self hide();
        }
        if ( self is_on( "No Recoil" ) )
            self player_recoilscaleon( 0 );
        if ( self is_on( "Rapid Fire" ) )
            self menu_rapidfire( 1 );
    }
}

// Nobody sees him: his body is gone for everyone else and bots never aim
// at him.
toggle_invisible()
{
    if ( self flip( "Invisible" ) )
    {
        self.ignoreme = 1;
        self hide();
        self say( "Invisible ^2ON" );
    }
    else
    {
        self.ignoreme = 0;
        self show();
        self say( "Invisible ^1OFF" );
    }
}

// Everyone jumps higher / falls slower (BO2's own jump_height and
// bg_gravity; 39 and 800 are normal).
toggle_super_jump()
{
    if ( self flip( "Super Jump" ) )
    {
        setdvar( "jump_height", 999 );
        self say( "Super Jump ^2ON" );
    }
    else
    {
        setdvar( "jump_height", 39 );
        self say( "Super Jump ^1OFF" );
    }
}

toggle_low_gravity()
{
    if ( self flip( "Low Gravity" ) )
    {
        setdvar( "bg_gravity", 200 );
        self say( "Low Gravity ^2ON" );
    }
    else
    {
        setdvar( "bg_gravity", 800 );
        self say( "Low Gravity ^1OFF" );
    }
}

toggle_no_recoil()
{
    if ( self flip( "No Recoil" ) )
    {
        self player_recoilscaleon( 0 );
        self say( "No Recoil ^2ON" );
    }
    else
    {
        self player_recoilscaleoff();
        self say( "No Recoil ^1OFF" );
    }
}

// Every gun fires three times as fast, single shot and burst guns too.
toggle_rapid_fire()
{
    on = self flip( "Rapid Fire" );
    self menu_rapidfire( on );
    if ( on )
        self say( "Rapid Fire ^2ON" );
    else
        self say( "Rapid Fire ^1OFF" );
}

// Each shot blows up where it lands (never right at your feet).
toggle_explosive()
{
    self notify( "mm_boom_off" );
    if ( self flip( "Explosive Bullets" ) )
    {
        self thread explosive_loop();
        self say( "Explosive Bullets ^2ON" );
    }
    else
        self say( "Explosive Bullets ^1OFF" );
}

explosive_loop()
{
    self endon( "disconnect" );
    self endon( "mm_boom_off" );
    for ( ;; )
    {
        self waittill( "weapon_fired" );
        eye = self geteye();
        fwd = anglestoforward( self getplayerangles() );
        trace = bullettrace( eye, eye + vectorscale( fwd, 20000 ), true, self );
        if ( trace["fraction"] >= 1 )
            continue;
        spot = trace["position"];
        if ( distance( spot, eye ) < 160 )
            continue;
        playfx( level.mm_fx, spot );
        radiusdamage( spot, 150, 150, 30, self, "MOD_GRENADE_SPLASH" );
    }
}

// Grenades, tacticals and equipment never run out.
toggle_equipment()
{
    self notify( "mm_equip_off" );
    if ( self flip( "Infinite Equipment" ) )
    {
        self thread equipment_loop();
        self say( "Infinite Equipment ^2ON" );
    }
    else
        self say( "Infinite Equipment ^1OFF" );
}

equipment_loop()
{
    self endon( "disconnect" );
    self endon( "mm_equip_off" );
    for ( ;; )
    {
        weapons = self getweaponslist();
        for ( i = 0; i < weapons.size; i++ )
        {
            if ( weaponinventorytype( weapons[i] ) == "offhand" )
                self givemaxammo( weapons[i] );
        }
        wait 0.5;
    }
}

// ---------------------------------------------------------------------------
// Trickshots
// ---------------------------------------------------------------------------

// A floor of Care Package crates under your feet, in the air or on the
// ground: 256 x 256, solid.
spawn_platform()
{
    if ( !isdefined( level.mm_platforms ) )
        level.mm_platforms = [];
    base = self.origin;
    yaw = self getplayerangles()[1];
    fwd = anglestoforward( ( 0, yaw, 0 ) );
    right = anglestoright( ( 0, yaw, 0 ) );
    floor = spawn( "script_model", base - ( 0, 0, 5 ) );
    floor setmodel( "collision_clip_wall_256x256x10" );
    floor.angles = ( 0, yaw, 90 );
    level.mm_platforms[level.mm_platforms.size] = floor;
    for ( r = 0; r < 4; r++ )
    {
        for ( c = 0; c < 4; c++ )
        {
            spot = base + vectorscale( fwd, ( r - 1.5 ) * 64 ) + vectorscale( right, ( c - 1.5 ) * 64 ) - ( 0, 0, 30 );
            crate = spawn( "script_model", spot );
            crate setmodel( "t6_wpn_supply_drop_ally" );
            crate.angles = ( 0, yaw, 0 );
            level.mm_platforms[level.mm_platforms.size] = crate;
        }
    }
    self say( "Platform spawned" );
}

remove_platforms()
{
    if ( !isdefined( level.mm_platforms ) || level.mm_platforms.size == 0 )
    {
        self say( "No platforms" );
        return;
    }
    for ( i = 0; i < level.mm_platforms.size; i++ )
    {
        if ( isdefined( level.mm_platforms[i] ) )
            level.mm_platforms[i] delete();
    }
    level.mm_platforms = [];
    self say( "Platforms removed" );
}

// A random gun drops at your feet, its ammo full: jump, grab it in the air
// (Use) and swap to it for the kill.
canswap()
{
    guns = strtok( "tar21_mp type95_mp sig556_mp sa58_mp hk416_mp scar_mp saritch_mp xm8_mp an94_mp mp7_mp pdw57_mp vector_mp insas_mp qcw05_mp evoskorpion_mp peacekeeper_mp 870mcs_mp saiga12_mp ksg_mp srm1216_mp mk48_mp qbb95_mp lsat_mp hamr_mp svu_mp dsr50_mp ballista_mp as50_mp", " " );
    w = guns[randomint( guns.size )];
    if ( self hasweapon( w ) )
        w = guns[( randomint( guns.size - 1 ) + 1 + mm_index( guns, w ) ) % guns.size];
    self giveweapon( w );
    self givemaxammo( w );
    item = self dropitem( w );
    if ( isdefined( item ) )
        self say( "Dropped a canswap" );
    else
        self say( "Couldn't drop a gun here" );
}

mm_index( list, word )
{
    for ( i = 0; i < list.size; i++ )
    {
        if ( list[i] == word )
            return i;
    }
    return 0;
}

// One kill from winning: your team's score (or yours in Free-for-All)
// goes to the score limit less one kill.
fast_last()
{
    if ( !isdefined( level.scorelimit ) || level.scorelimit <= 0 )
    {
        self say( "This match has no score limit" );
        return;
    }
    per = level.teamscoreperkill;
    if ( !isdefined( per ) || per <= 0 )
        per = 1;
    last = level.scorelimit - per;
    if ( level.teambased )
        maps\mp\gametypes\_globallogic_score::_setteamscore( self.pers["team"], last );
    else
        self maps\mp\gametypes\_globallogic_score::setpointstowin( last );
    self say( "^3Fast last^7: one kill to win" );
}

// ---------------------------------------------------------------------------
// Game
// ---------------------------------------------------------------------------

// The match clock stops (BO2 hides it while it is stopped).
toggle_pause_clock()
{
    if ( self flip( "Pause the Clock" ) )
    {
        maps\mp\gametypes\_globallogic_utils::pausetimer();
        self say( "Clock ^2paused" );
    }
    else
    {
        maps\mp\gametypes\_globallogic_utils::resumetimer();
        self say( "Clock ^1running" );
    }
}

// No score ends the match.
toggle_unlimited_score()
{
    if ( self flip( "Unlimited Score" ) )
    {
        // BO2 reads the limit back from the game type setting every second.
        if ( level.scorelimit > 0 )
            level.mm_scorelimit = level.scorelimit;
        setgametypesetting( "scoreLimit", 0 );
        level.scorelimit = 0;
        self say( "Unlimited Score ^2ON" );
    }
    else
    {
        if ( isdefined( level.mm_scorelimit ) )
        {
            setgametypesetting( "scoreLimit", level.mm_scorelimit );
            level.scorelimit = level.mm_scorelimit;
        }
        self say( "Unlimited Score ^1OFF" );
    }
}

// The whole game at half speed.
toggle_slow_motion()
{
    if ( self flip( "Slow Motion" ) )
    {
        setslowmotion( 1, 0.5, 0 );
        self say( "Slow Motion ^2ON" );
    }
    else
    {
        setslowmotion( 0.5, 1, 0 );
        self say( "Slow Motion ^1OFF" );
    }
}

// Load another map; same mode, same bots.
change_map( name )
{
    self say( "Loading..." );
    map( name );
}

// The same map again in another mode.
change_mode( mode )
{
    self say( "Changing mode..." );
    setdvar( "g_gametype", mode );
    map( getdvar( "mapname" ), mode );
}

// ---------------------------------------------------------------------------
// Bots (more)
// ---------------------------------------------------------------------------

// Bots still run around but never shoot anyone.
toggle_bots_no_shoot()
{
    self notify( "mm_noshoot_off" );
    if ( self flip( "Bots Don't Shoot" ) )
    {
        self thread bots_no_shoot_loop();
        self say( "Bots Don't Shoot ^2ON" );
    }
    else
    {
        players = getplayers();
        for ( i = 0; i < players.size; i++ )
        {
            if ( players[i] istestclient() )
                players[i].ignoreall = 0;
        }
        self say( "Bots Don't Shoot ^1OFF" );
    }
}

// Bots that join later stop shooting too.
bots_no_shoot_loop()
{
    self endon( "disconnect" );
    self endon( "mm_noshoot_off" );
    for ( ;; )
    {
        players = getplayers();
        for ( i = 0; i < players.size; i++ )
        {
            if ( players[i] istestclient() )
                players[i].ignoreall = 1;
        }
        wait 0.5;
    }
}

kill_bots()
{
    players = getplayers();
    n = 0;
    for ( i = 0; i < players.size; i++ )
    {
        p = players[i];
        if ( !p istestclient() || !isalive( p ) )
            continue;
        // Friendly fire would shield your own team's bots: they just drop.
        if ( level.teambased && p.pers["team"] == self.pers["team"] )
            p dodamage( p.health + 1000, p.origin );
        else
            p dodamage( p.health + 1000, p.origin, self, self, "none", "MOD_RIFLE_BULLET" );
        n++;
    }
    self say( "Killed " + n + " bots" );
}

// Every bot leaves, and the game stops bringing new ones (Add Bot still
// works).
kick_bots()
{
    level.menu_nobots = 1;
    players = getplayers();
    n = 0;
    for ( i = 0; i < players.size; i++ )
    {
        if ( players[i] istestclient() )
        {
            kick( players[i] getentitynumber() );
            n++;
        }
    }
    self say( "Kicked " + n + " bots" );
}

// ---------------------------------------------------------------------------
// Map Test
// ---------------------------------------------------------------------------

// "x,y,z Name" = look at that spot from a floor near it;
// "x,y,z,yaw,pitch Name" = stand right there. Nuketown 2025 for now.
mt_spot_list()
{
    if ( getdvar( "mapname" ) != "mp_nuketown_2020" )
        return "0,0,0 Middle of the map";
    return "410,-1054,-29 Car 1 (south road);32,-472,-26 Car 2 (middle);37,-895,-36 Car 3 (south);-690,9,-27 Car 4 (west);-59,804,-36 Car 5 (north);-239,-605,-30 Gate sign (left);305,-605,-30 Gate sign (right);-235,522,-40 Population sign (Atari TV);-195,419,-58,125,0 Atari TV use spot;-523,297,145 Breakable window (west);559,67,146 Breakable window (east);0,0,-60 Middle of the map";
}

mt_go( key )
{
    v = strtok( key, "," );
    target = ( float( v[0] ), float( v[1] ), float( v[2] ) );
    if ( v.size > 3 )
    {
        self setorigin( target );
        self setvelocity( ( 0, 0, 0 ) );
        self setplayerangles( ( float( v[4] ), float( v[3] ), 0 ) );
        self say( "There" );
        return;
    }
    if ( self mt_stand( target, 150, -1000 ) )
        self say( "Looking at it" );
    else
        self say( "No floor with a view of it" );
}

// A floor near the spot with a clear view of it and room to stand, tried on
// rings from r0 out. floorz > -1000: the floor must be within 40 of it.
mt_stand( target, r0, floorz )
{
    for ( r = r0; r <= r0 + 400; r += 50 )
    {
        for ( k = 0; k < 12; k++ )
        {
            spot = target + vectorscale( anglestoforward( ( 0, k * 30, 0 ) ), r );
            top = target[2] + 48;
            if ( top > 250 )
                top = 250;
            down = bullettrace( ( spot[0], spot[1], top ), ( spot[0], spot[1], top - 700 ), false, self );
            if ( down["fraction"] >= 1 )
                continue;
            g = down["position"];
            if ( g[2] < -150 )
                continue;
            if ( floorz > -1000 && abs( g[2] - floorz ) > 40 )
                continue;
            if ( bullettrace( g + ( 0, 0, 4 ), g + ( 0, 0, 72 ), false, self )["fraction"] < 1 )
                continue;
            eye = g + ( 0, 0, 60 );
            if ( distance( bullettrace( eye, target, false, self )["position"], target ) > 24 )
                continue;
            self setorigin( g );
            self setvelocity( ( 0, 0, 0 ) );
            self setplayerangles( vectortoangles( target - eye ) );
            return true;
        }
    }
    return false;
}

mt_crosshair()
{
    eye = self geteye();
    fwd = anglestoforward( self getplayerangles() );
    return bullettrace( eye, eye + vectorscale( fwd, 20000 ), false, self );
}

// Where the crosshair lands, and the nearest map object to it.
mt_look()
{
    trace = self mt_crosshair();
    if ( trace["fraction"] >= 1 )
    {
        self say( "Sky" );
        return;
    }
    spot = trace["position"];
    self iprintln( "At " + int( spot[0] ) + " " + int( spot[1] ) + " " + int( spot[2] ) + ", " + int( distance( spot, self geteye() ) ) + " away" );
    best = undefined;
    bestd = 96;
    classes = strtok( "script_model script_brushmodel", " " );
    for ( c = 0; c < classes.size; c++ )
    {
        ents = getentarray( classes[c], "classname" );
        for ( i = 0; i < ents.size; i++ )
        {
            d = distance( ents[i].origin, spot );
            if ( d < bestd )
            {
                best = ents[i];
                bestd = d;
            }
        }
    }
    if ( !isdefined( best ) )
    {
        self say( "No map object near it" );
        return;
    }
    text = best.classname;
    if ( isdefined( best.destructibledef ) )
        text = text + " " + best.destructibledef;
    else if ( isdefined( best.model ) )
        text = text + " " + best.model;
    if ( isdefined( best.targetname ) )
        text = text + " (" + best.targetname + ")";
    if ( isdefined( best.health ) )
        text = text + " health " + best.health;
    self say( text );
}

// A blast like a car's own explosion (256 radius, 300 to 75 damage).
mt_blast()
{
    trace = self mt_crosshair();
    if ( trace["fraction"] >= 1 )
    {
        self say( "Aim at something" );
        return;
    }
    playfx( level.mm_fx, trace["position"] );
    radiusdamage( trace["position"], 256, 300, 75, self, "MOD_EXPLOSIVE" );
    self say( "Blast" );
}

mt_rocket()
{
    trace = self mt_crosshair();
    eye = self geteye();
    fwd = anglestoforward( self getplayerangles() );
    magicbullet( "smaw_mp", eye + vectorscale( fwd, 40 ), trace["position"], self );
    self say( "Rocket" );
}

mt_mannequins()
{
    out = [];
    ents = getentarray( "destructible", "targetname" );
    for ( i = 0; i < ents.size; i++ )
    {
        if ( isdefined( ents[i].destructibledef ) && issubstr( ents[i].destructibledef, "male" ) )
            out[out.size] = ents[i];
    }
    return out;
}

// Each pick goes to the next mannequin on the map and aims at its head.
mt_next_head()
{
    m = mt_mannequins();
    if ( m.size == 0 )
    {
        self say( "No mannequins on this map" );
        return;
    }
    if ( !isdefined( self.mm.head ) || self.mm.head >= m.size )
        self.mm.head = 0;
    e = m[self.mm.head];
    self.mm.head++;
    head = e getpointinbounds( 0, 0, 0.86 );
    if ( self mt_stand( head, 70, e.origin[2] ) )
        self say( "Mannequin " + self.mm.head + " of " + m.size );
    else
        self say( "Mannequin " + self.mm.head + ": no spot with a view" );
}

mt_egg()
{
    if ( !isdefined( level.mannequin_count ) )
    {
        self say( "No mannequin egg on this map" );
        return;
    }
    text = "Heads off " + level.headless_mannequin_count + " of " + level.mannequin_count;
    if ( isdefined( level.mannequin_time ) )
    {
        left = int( 120 - ( gettime() - level.mannequin_time ) / 1000 );
        if ( left < 0 )
            left = 0;
        text = text + ", " + left + " s left";
    }
    self say( text );
}

mt_end()
{
    self say( "Ending the match" );
    thread maps\mp\gametypes\_globallogic::forceend();
}
