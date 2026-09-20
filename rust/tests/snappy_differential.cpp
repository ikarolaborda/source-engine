// Cross-decodes Source-tagged Snappy with the retained native codec. Encoders
// may choose different matches; no byte-identical compression claim is made.
#include "public/rust/source_abi.h"
#include "tier1/snappy.h"
#include <algorithm>
#include <cstdio>
#include <cstring>
#include <vector>

static SourceAbiSlice Slice( const void *data, size_t length )
{
	SourceAbiSlice result = { static_cast<const uint8_t *>( data ), static_cast<uint64_t>( length ) };
	return result;
}

static uint32_t Random( uint32_t &state )
{
	state ^= state << 13;
	state ^= state >> 17;
	state ^= state << 5;
	return state;
}

static bool Check( const std::vector<uint8_t> &input, size_t index )
{
	uint64_t bound = 0;
	if ( source_compress_snappy_max_size( input.size(), &bound ) != SOURCE_ABI_OK ||
		bound != snappy::MaxCompressedLength( input.size() ) + 4 )
		return false;
	// Give empty vectors a nonnull pointer for the native API.
	const char *plain = input.empty() ? "" : reinterpret_cast<const char *>( input.data() );
	std::vector<uint8_t> native( bound );
	std::memcpy( native.data(), "SNAP", 4 );
	size_t nativeLength = 0;
	snappy::RawCompress( plain, input.size(), reinterpret_cast<char *>( native.data() + 4 ), &nativeLength );
	native.resize( nativeLength + 4 );
	std::vector<uint8_t> rust( bound, 0x5a );
	uint64_t rustLength = 0;
	if ( source_compress_snappy_compress( Slice( plain, input.size() ), rust.data(), bound, &rustLength ) != SOURCE_ABI_OK || rustLength > bound )
		return false;
	rust.resize( rustLength );
	std::vector<uint8_t> decoded( input.size() + 1, 0x5a );
	uint64_t length = 0;
	if ( source_compress_buffer_decompress( Slice( native.data(), native.size() ), decoded.data(), input.size(), &length ) != SOURCE_ABI_OK ||
		length != input.size() || std::memcmp( decoded.data(), plain, input.size() ) || decoded.back() != 0x5a )
	{
		std::printf( "native -> Rust failed at corpus %zu\n", index );
		return false;
	}
	std::fill( decoded.begin(), decoded.end(), 0x5a );
	if ( !snappy::RawUncompress( reinterpret_cast<const char *>( rust.data() + 4 ), rust.size() - 4,
		reinterpret_cast<char *>( decoded.data() ) ) ||
		std::memcmp( decoded.data(), plain, input.size() ) || decoded.back() != 0x5a )
	{
		std::printf( "Rust -> native failed at corpus %zu\n", index );
		return false;
	}
	if ( source_compress_buffer_actual_size( Slice( native.data(), native.size() ), &length ) != SOURCE_ABI_OK || length != input.size() )
		return false;
	// Output sizing must not write even a prefix when the destination is short.
	std::vector<uint8_t> shortOutput( rustLength, 0x5a );
	if ( source_compress_snappy_compress( Slice( plain, input.size() ), shortOutput.data(), rustLength - 1, &length ) != SOURCE_ABI_BUFFER_TOO_SMALL || length != rustLength )
		return false;
	for ( uint8_t byte : shortOutput ) if ( byte != 0x5a ) return false;
	return true;
}

int main()
{
	size_t count = 0;
	uint32_t random = 0x1839173;
	std::vector<size_t> sizes;
	for ( size_t size = 0; size <= 512; ++size ) sizes.push_back( size );
	for ( size_t size : { 2047, 2048, 2049, 32767, 32768, 32769, 65535, 65536, 65537, 100000, 262144 } ) sizes.push_back( size );
	for ( size_t size : sizes )
	{
		for ( int pattern = 0; pattern < 4; ++pattern )
		{
			std::vector<uint8_t> input( size );
			for ( size_t i = 0; i < size; ++i )
				input[i] = pattern == 0 ? 0x5a : pattern == 1 ? i % 251 :
					pattern == 2 ? Random( random ) >> 24 : ( i % 37 ? 0 : Random( random ) >> 24 );
			if ( !Check( input, count++ ) ) return 1;
		}
	}

	// Compare validation of deterministic arbitrary/mutated raw token streams.
	// The native length must be bounded before invoking its allocating decoder.
	size_t malformed = 0;
	for ( size_t i = 0; i < 10000; ++i )
	{
		std::vector<uint8_t> raw( 1 + Random( random ) % 128 );
		for ( uint8_t &byte : raw ) byte = Random( random ) >> 24;
		if ( i % 2 == 0 ) raw[0] = static_cast<uint8_t>( i % 128 );
		size_t declared = 0;
		const char *data = reinterpret_cast<const char *>( raw.data() );
		if ( !snappy::GetUncompressedLength( data, raw.size(), &declared ) || declared > 1048576 ) continue;
		std::vector<uint8_t> native( declared + 1, 0x5a );
		const bool accepted = snappy::RawUncompress( data, raw.size(), reinterpret_cast<char *>( native.data() ) );
		std::vector<uint8_t> tagged( raw.size() + 4 );
		std::memcpy( tagged.data(), "SNAP", 4 );
		std::memcpy( tagged.data() + 4, raw.data(), raw.size() );
		std::vector<uint8_t> rust( declared + 1, 0x5a );
		uint64_t length = 0;
		const SourceAbiStatus status = source_compress_buffer_decompress( Slice( tagged.data(), tagged.size() ), rust.data(), declared, &length );
		if ( accepted != ( status == SOURCE_ABI_OK ) || ( accepted && ( length != declared || native != rust ) ) || rust.back() != 0x5a )
		{
			std::printf( "validation differs at mutated stream %zu: native %d Rust %d\n", i, accepted, status );
			return 2;
		}
		if ( !accepted )
		{
			for ( uint8_t byte : rust ) if ( byte != 0x5a ) return 3;
			++malformed;
		}
	}
	std::printf( "Snappy differential: %zu buffers cross-decoded, %zu malformed streams rejected by both\n", count, malformed );
	return 0;
}
