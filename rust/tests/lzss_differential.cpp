// Compares the Rust LZSS codec against the native CLZSS it replaces.
//
// A save written by one side has to be readable by the other, so agreeing on
// the format is not enough: the encoders have to agree byte for byte, because
// a difference in which match either one picks produces a different file for
// the same input. This drives both over the same corpus and requires
// identical output, then requires each side to decode what the other
// produced.

#include "public/rust/source_abi.h"

// platform.h first, because the codec's header is written against the
// engine's own integer types and inlining macros.
#include "tier0/platform.h"
#include "tier1/lzss.h"

#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

namespace
{

SourceAbiSlice Slice( const void *data, size_t length )
{
	SourceAbiSlice slice;
	slice.data = reinterpret_cast<const uint8_t *>( data );
	slice.length = static_cast<uint64_t>( length );
	return slice;
}

// Deterministic pseudo-random bytes, so a mismatch is reproducible without
// carrying a corpus around.
uint32_t Mix( uint32_t state )
{
	state ^= state << 13;
	state ^= state >> 17;
	state ^= state << 5;
	return state;
}

// Inputs chosen for the cases where the two encoders could disagree: run
// lengths either side of the three-byte minimum and the sixteen-byte
// lookahead, distances either side of the window, token counts either side of
// a group of eight, and data with no matches at all.
std::vector<std::vector<uint8_t> > BuildCorpus()
{
	std::vector<std::vector<uint8_t> > corpus;

	static const char *kProse =
		"the quick brown fox jumps over the lazy dog; "
		"the quick brown fox jumps over the lazy dog; "
		"the quick brown fox jumps over the lazy dog.";
	corpus.push_back( std::vector<uint8_t>( kProse, kProse + std::strlen( kProse ) + 1 ) );

	// Every length across several groups, so the final partial group and its
	// end marker are exercised at every remainder.
	for ( size_t length = 17; length < 300; ++length )
	{
		std::vector<uint8_t> repeating( length, 0x5a );
		corpus.push_back( repeating );

		std::vector<uint8_t> cyclic( length );
		for ( size_t index = 0; index < length; ++index )
			cyclic[index] = static_cast<uint8_t>( index % 7 );
		corpus.push_back( cyclic );

		std::vector<uint8_t> incompressible( length );
		uint32_t state = static_cast<uint32_t>( length ) + 1u;
		for ( size_t index = 0; index < length; ++index )
		{
			state = Mix( state );
			incompressible[index] = static_cast<uint8_t>( state >> 24 );
		}
		corpus.push_back( incompressible );
	}

	// Long enough to wrap the window several times, which is where the two
	// match indexes could start evicting different positions.
	for ( size_t copies = 1; copies <= 4; ++copies )
	{
		std::vector<uint8_t> wrapping;
		for ( size_t index = 0; index < 4096 * copies; ++index )
			wrapping.push_back( static_cast<uint8_t>( index % 251 ) );
		corpus.push_back( wrapping );

		// Two identical halves, so every match in the second half reaches
		// back exactly one window or further.
		std::vector<uint8_t> doubled( wrapping );
		doubled.insert( doubled.end(), wrapping.begin(), wrapping.end() );
		corpus.push_back( doubled );
	}

	// Runs of each length around the lookahead limit, back to back, so the
	// encoders have to agree on when a run stops being worth a reference.
	std::vector<uint8_t> runs;
	for ( size_t length = 1; length <= 40; ++length )
		for ( size_t repeat = 0; repeat < length; ++repeat )
			runs.push_back( static_cast<uint8_t>( length ) );
	corpus.push_back( runs );

	// Structured data resembling a save: mostly zeroes with sparse values.
	std::vector<uint8_t> sparse( 20000, 0 );
	uint32_t state = 12345u;
	for ( size_t index = 0; index < sparse.size(); index += 37 )
	{
		state = Mix( state );
		sparse[index] = static_cast<uint8_t>( state >> 24 );
	}
	corpus.push_back( sparse );

	return corpus;
}

std::string Hex( const uint8_t *data, size_t length )
{
	static const char *kDigits = "0123456789abcdef";
	std::string text;
	for ( size_t index = 0; index < length; ++index )
	{
		text.push_back( kDigits[data[index] >> 4] );
		text.push_back( kDigits[data[index] & 0x0f] );
	}
	return text;
}

} // namespace

int main()
{
	const std::vector<std::vector<uint8_t> > corpus = BuildCorpus();
	size_t compared = 0;
	size_t bothDeclined = 0;

	// Both windows the engine uses: the default for generic buffers and the
	// smaller one the save system passes. The window changes which matches
	// are reachable, so agreeing at one size says nothing about the other.
	static const unsigned int kWindows[] = { 4096, 2048 };

	for ( size_t entry = 0; entry < corpus.size() * 2; ++entry )
	{
		const size_t index = entry / 2;
		const unsigned int window = kWindows[entry % 2];
		const std::vector<uint8_t> &input = corpus[index];

		CLZSS lzss( static_cast<int>( window ) );
		std::vector<uint8_t> nativeBuffer( input.size() + 16 );
		unsigned int nativeLength = 0;
		const bool nativeCompressed = lzss.CompressNoAlloc( input.data(),
			static_cast<int>( input.size() ), nativeBuffer.data(), &nativeLength ) != NULL;

		std::vector<uint8_t> rustBuffer( input.size() + 16 );
		uint64_t rustLength = 0;
		const SourceAbiStatus rustStatus = source_compress_lzss_compress(
			Slice( input.data(), input.size() ), window, rustBuffer.data(),
			static_cast<uint64_t>( rustBuffer.size() ), &rustLength );

		// Declining has to be mutual. If one side compresses input the other
		// gives up on, the engine's choice of whether to store a buffer
		// compressed would change with which codec wrote it.
		if ( !nativeCompressed || rustStatus == SOURCE_ABI_DECLINED )
		{
			if ( nativeCompressed || rustStatus != SOURCE_ABI_DECLINED )
			{
				std::printf( "corpus %zu window %u (%zu bytes): native %s but Rust %s\n", index,
					window, input.size(), nativeCompressed ? "compressed" : "declined",
					rustStatus == SOURCE_ABI_DECLINED ? "declined" : "compressed" );
				return 1;
			}
			++bothDeclined;
			continue;
		}

		if ( rustStatus != SOURCE_ABI_OK )
		{
			std::printf( "corpus %zu window %u (%zu bytes): Rust compress returned %d\n", index,
				window, input.size(), rustStatus );
			return 2;
		}

		if ( rustLength != nativeLength ||
			std::memcmp( rustBuffer.data(), nativeBuffer.data(),
				static_cast<size_t>( nativeLength ) ) != 0 )
		{
			std::printf( "corpus %zu window %u (%zu bytes): native %u bytes, Rust %llu bytes\n",
				index, window, input.size(), nativeLength, ( unsigned long long )rustLength );
			std::printf( "  native %s\n", Hex( nativeBuffer.data(),
				nativeLength < 64 ? nativeLength : 64 ).c_str() );
			std::printf( "  rust   %s\n", Hex( rustBuffer.data(),
				rustLength < 64 ? ( size_t )rustLength : 64 ).c_str() );
			return 3;
		}

		// Each decoder has to read the bytes the other side just agreed on,
		// and land on the original.
		uint64_t declared = 0;
		if ( source_compress_lzss_actual_size( Slice( nativeBuffer.data(), nativeLength ),
			&declared ) != SOURCE_ABI_OK || declared != input.size() )
		{
			std::printf( "corpus %zu window %u: Rust read a declared size of %llu\n", index, window,
				( unsigned long long )declared );
			return 4;
		}

		std::vector<uint8_t> rustPlain( input.size() );
		uint64_t plainLength = 0;
		if ( source_compress_lzss_decompress( Slice( nativeBuffer.data(), nativeLength ),
			rustPlain.data(), static_cast<uint64_t>( rustPlain.size() ), &plainLength ) !=
				SOURCE_ABI_OK ||
			plainLength != input.size() || rustPlain != input )
		{
			std::printf( "corpus %zu window %u: Rust did not decode the native stream\n", index, window );
			return 5;
		}

		std::vector<uint8_t> nativePlain( input.size() + 16 );
		const unsigned int nativePlainLength = lzss.SafeUncompress( rustBuffer.data(),
			static_cast<unsigned int>( rustLength ), nativePlain.data(),
			static_cast<unsigned int>( nativePlain.size() ) );
		if ( nativePlainLength != input.size() ||
			std::memcmp( nativePlain.data(), input.data(), input.size() ) != 0 )
		{
			std::printf( "corpus %zu window %u: native did not decode the Rust stream (%u bytes)\n",
				index, window, nativePlainLength );
			return 6;
		}

		++compared;
	}

	std::printf( "LZSS differential: %zu buffers byte-identical, %zu declined by both\n",
		compared, bothDeclined );
	return 0;
}
