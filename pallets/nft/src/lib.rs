// This file is part of Metaverse.Network & Bit.Country.

// Copyright (C) 2020-2022 Metaverse.Network & Bit.Country .
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// This pallet use The Open Runtime Module Library (ORML) which is a community maintained collection
// of Substrate runtime modules. Thanks to all contributors of orml.
// Ref: https://github.com/open-web3-stack/open-runtime-module-library

#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::string_lit_as_bytes)]
#![allow(clippy::unnecessary_cast)]
#![allow(clippy::unused_unit)]
#![allow(clippy::upper_case_acronyms)]
#![cfg_attr(not(feature = "std"), no_std)]

use codec::Encode;
use frame_support::traits::ExistenceRequirement::KeepAlive;
use frame_support::traits::Len;
use frame_support::{
	dispatch::{DispatchResult, DispatchResultWithPostInfo},
	ensure,
	pallet_prelude::*,
	traits::{Currency, ExistenceRequirement, Get, LockIdentifier, ReservableCurrency},
	transactional, PalletId,
};
use frame_system::pallet_prelude::*;
use orml_nft::{ClassInfo, ClassInfoOf, Classes, Pallet as NftModule, TokenInfo, TokenInfoOf, TokenMetadataOf, Tokens};
use sp_runtime::traits::Saturating;
use sp_runtime::Perbill;
use sp_runtime::{
	traits::{AccountIdConversion, One},
	DispatchError,
};
use sp_std::prelude::*;
use sp_std::vec::Vec;

use auction_manager::{Auction, CheckAuctionItemHandler};
pub use pallet::*;
pub use primitive_traits::{Attributes, NFTTrait, NftClassData, NftGroupCollectionData, NftMetadata, TokenType};
use primitive_traits::{CollectionType, NftAssetData, NftClassDataV1, PreSignedMint};
use primitives::{AssetId, ClassId, GroupCollectionId, ItemId, TokenId};
pub use weights::WeightInfo;

#[cfg(feature = "runtime-benchmarks")]
pub mod benchmarking;
#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;
mod utils;

pub mod weights;

const TIMECAPSULE_ID: LockIdentifier = *b"bctimeca";

#[derive(codec::Encode, codec::Decode, Clone, RuntimeDebug, PartialEq)]
pub enum StorageVersion {
	V0,
	V1,
}

#[frame_support::pallet]
pub mod pallet {
	use orml_traits::{MultiCurrency, MultiCurrencyExtended};
	use sp_runtime::traits::{CheckedSub, IdentifyAccount, Verify};
	use sp_runtime::ArithmeticError;

	use primitive_traits::{
		CollectionType, NftAssetData, NftGroupCollectionData, NftMetadata, PreSignedMint, TokenType,
	};
	use primitives::FungibleTokenId;

	use super::*;

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(PhantomData<T>);

	#[pallet::config]
	pub trait Config:
		frame_system::Config
		+ orml_nft::Config<TokenData = NftAssetData<BalanceOf<Self>>, ClassData = NftClassData<BalanceOf<Self>>>
		+ pallet_proxy::Config
	{
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		/// The data deposit per byte to calculate fee
		/// Default minting price per NFT token
		#[pallet::constant]
		type AssetMintingFee: Get<BalanceOf<Self>>;
		/// Default minting price per NFT token class
		#[pallet::constant]
		type ClassMintingFee: Get<BalanceOf<Self>>;
		/// Treasury
		#[pallet::constant]
		type Treasury: Get<PalletId>;
		/// Currency type for reserve/unreserve balance
		type Currency: Currency<Self::AccountId> + ReservableCurrency<Self::AccountId>;
		/// NFT Module Id
		#[pallet::constant]
		type PalletId: Get<PalletId>;
		/// Weight info
		type WeightInfo: WeightInfo;
		/// Auction Handler
		type AuctionHandler: Auction<Self::AccountId, BlockNumberFor<Self>> + CheckAuctionItemHandler<BalanceOf<Self>>;
		/// Max transfer batch
		#[pallet::constant]
		type MaxBatchTransfer: Get<u32>;
		/// Max batch minting
		#[pallet::constant]
		type MaxBatchMinting: Get<u32>;
		/// Max metadata length
		#[pallet::constant]
		type MaxMetadata: Get<u32>;
		/// Multi currency type for promotion incentivization
		type MultiCurrency: MultiCurrencyExtended<
			Self::AccountId,
			CurrencyId = FungibleTokenId,
			Balance = BalanceOf<Self>,
		>;
		/// Fungible token id for promotion incentive
		#[pallet::constant]
		type MiningResourceId: Get<FungibleTokenId>;
		/// Storage deposit free charged when saving data into the blockchain.
		/// The fee will be unreserved after the storage is freed.
		#[pallet::constant]
		type StorageDepositFee: Get<BalanceOf<Self>>;

		/// Off-Chain signature type.
		///
		/// Can verify whether an `Self::OffchainPublic` created a signature.
		type OffchainSignature: Verify<Signer = Self::OffchainPublic> + Parameter;

		/// Off-Chain public key.
		///
		/// Must identify as an on-chain `Self::AccountId`.
		type OffchainPublic: IdentifyAccount<AccountId = Self::AccountId>;
	}

	pub type ClassIdOf<T> = <T as orml_nft::Config>::ClassId;
	pub type TokenIdOf<T> = <T as orml_nft::Config>::TokenId;
	pub type BalanceOf<T> =
		<<T as orml_nft::Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;
	/// A type alias for the pre-signed minting configuration for a specified collection.
	pub(super) type PreSignedMintOf<T> = PreSignedMint<
		<T as orml_nft::Config>::ClassId,
		<T as orml_nft::Config>::TokenId,
		<T as frame_system::Config>::AccountId,
		BlockNumberFor<T>,
		BalanceOf<T>,
	>;

	#[pallet::storage]
	#[pallet::getter(fn get_group_collection)]
	/// Stores NFT group collection data
	pub(super) type GroupCollections<T: Config> =
		StorageMap<_, Blake2_128Concat, GroupCollectionId, NftGroupCollectionData, OptionQuery>;

	#[pallet::storage]
	#[pallet::getter(fn get_class_collection)]
	/// Stores group collection IDs for every class
	pub(super) type ClassDataCollection<T: Config> =
		StorageMap<_, Blake2_128Concat, ClassIdOf<T>, GroupCollectionId, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn next_group_collection_id)]
	/// Track the next group collection ID
	pub(super) type NextGroupCollectionId<T: Config> = StorageValue<_, u64, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn all_nft_collection_count)]
	/// Track the total NFT group collection IDs
	pub(super) type AllNftGroupCollection<T: Config> = StorageValue<_, u64, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn next_asset_id)]
	/// Track the next asset ID
	pub(super) type NextAssetId<T: Config> = StorageValue<_, AssetId, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn get_asset_supporters)]
	/// Stores list of supporter accounts for every NFT assets
	pub(super) type AssetSupporters<T: Config> =
		StorageMap<_, Blake2_128Concat, (ClassIdOf<T>, TokenIdOf<T>), Vec<T::AccountId>, OptionQuery>;

	#[pallet::storage]
	#[pallet::getter(fn get_promotion_enabled)]
	/// Tracks if promotion is enabled
	pub(super) type PromotionEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn get_locked_collection)]
	/// Index locked collections by class ID
	pub(super) type LockedCollection<T: Config> = StorageMap<_, Blake2_128Concat, ClassIdOf<T>, (), OptionQuery>;

	#[pallet::storage]
	#[pallet::getter(fn reserved_stackable_nft_balances)]
	/// Reserved stackable nft balance
	pub(super) type ReservedStackableNftBalance<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat,
		T::AccountId,
		Twox64Concat,
		(ClassIdOf<T>, TokenIdOf<T>),
		BalanceOf<T>,
		ValueQuery,
	>;

	#[pallet::genesis_config]
	#[derive(frame_support::DefaultNoBound)]
	pub struct GenesisConfig<T> {
		pub _config: PhantomData<T>,
	}

	#[pallet::genesis_build]
	impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
		fn build(&self) {
			// Pre-mint group collection for lands
			let land_collection_data = NftGroupCollectionData {
				name: "MetaverseLands".as_bytes().to_vec(),
				properties: "MetaverseId;Coordinates".as_bytes().to_vec(),
			};
			let land_collection_id = <Pallet<T>>::next_group_collection_id();
			<GroupCollections<T>>::insert(land_collection_id, land_collection_data);
			<NextGroupCollectionId<T>>::set(land_collection_id + 1);
			<AllNftGroupCollection<T>>::set(land_collection_id + 1);
			<Pallet<T>>::deposit_event(Event::NewNftCollectionCreated(land_collection_id));

			// Pre-mint group collection for estates
			let estate_collection_data = NftGroupCollectionData {
				name: "MetaverseEstate".as_bytes().to_vec(),
				properties: "MetaverseId;EstateId".as_bytes().to_vec(),
			};
			let estate_collection_id = <Pallet<T>>::next_group_collection_id();
			<GroupCollections<T>>::insert(estate_collection_id, estate_collection_data);
			<NextGroupCollectionId<T>>::set(estate_collection_id + 1);
			<AllNftGroupCollection<T>>::set(estate_collection_id + 1);
			<Pallet<T>>::deposit_event(Event::NewNftCollectionCreated(estate_collection_id));
		}
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub (crate) fn deposit_event)]
	pub enum Event<T: Config> {
		/// New NFT Group Collection created
		NewNftCollectionCreated(GroupCollectionId),
		/// New NFT Collection/Class created
		NewNftClassCreated(<T as frame_system::Config>::AccountId, ClassIdOf<T>),
		/// Emit event when new nft minted - show the first and last asset mint
		NewNftMinted(
			(ClassIdOf<T>, TokenIdOf<T>),
			(ClassIdOf<T>, TokenIdOf<T>),
			<T as frame_system::Config>::AccountId,
			ClassIdOf<T>,
			u32,
			TokenIdOf<T>,
		),
		/// Emit event when new nft minted - show the first and last asset mint
		NewStackableNftMinted(
			<T as frame_system::Config>::AccountId,
			ClassIdOf<T>,
			TokenIdOf<T>,
			BalanceOf<T>,
		),
		/// Emit event when new time capsule minted
		NewTimeCapsuleMinted(
			(ClassIdOf<T>, TokenIdOf<T>),
			<T as frame_system::Config>::AccountId,
			ClassIdOf<T>,
			TokenIdOf<T>,
			BlockNumberFor<T>,
			Vec<u8>,
		),
		/// Successfully transfer NFT
		TransferedNft(
			<T as frame_system::Config>::AccountId,
			<T as frame_system::Config>::AccountId,
			TokenIdOf<T>,
			(ClassIdOf<T>, TokenIdOf<T>),
		),
		/// Successfully transfer NFT
		TransferedStackableNft(
			<T as frame_system::Config>::AccountId,
			<T as frame_system::Config>::AccountId,
			(ClassIdOf<T>, TokenIdOf<T>),
			BalanceOf<T>,
		),
		/// Successfully force transfer NFT
		ForceTransferredNft(
			<T as frame_system::Config>::AccountId,
			<T as frame_system::Config>::AccountId,
			TokenIdOf<T>,
			(ClassIdOf<T>, TokenIdOf<T>),
		),
		/// Successfully updated class total issuance
		ClassTotalIssuanceUpdated(ClassIdOf<T>, TokenIdOf<T>),
		/// Signed on NFT
		SignedNft(TokenIdOf<T>, <T as frame_system::Config>::AccountId),
		/// Promotion enabled
		PromotionEnabled(bool),
		/// Burn NFT
		BurnedNft((ClassIdOf<T>, TokenIdOf<T>)),
		/// Executed NFT
		ExecutedNft(AssetId),
		/// Scheduled time capsule
		ScheduledTimeCapsule(AssetId, Vec<u8>, BlockNumberFor<T>),
		/// Collection is locked
		CollectionLocked(ClassIdOf<T>),
		/// Collection is unlocked
		CollectionUnlocked(ClassIdOf<T>),
		/// Hard limit is set
		HardLimitSet(ClassIdOf<T>),
		/// Class funds are withdrawn
		ClassFundsWithdrawn(ClassIdOf<T>),
		/// NFT is unlocked
		NftUnlocked(ClassIdOf<T>, TokenIdOf<T>),
		/// Successfully updated royalty fee
		ClassRoyaltyFeeUpdated(ClassIdOf<T>, Perbill),
		// New proxy NFT minted
		NewProxyNftMinted(
			(ClassIdOf<T>, TokenIdOf<T>),
			(ClassIdOf<T>, TokenIdOf<T>),
			<T as frame_system::Config>::AccountId,
			ClassIdOf<T>,
			u32,
			TokenIdOf<T>,
			<T as frame_system::Config>::AccountId,
		),
	}

	#[pallet::error]
	pub enum Error<T> {
		/// Attempted to initialize the metaverse after it had already been initialized.
		AlreadyInitialized,
		/// Asset Info not found
		AssetInfoNotFound,
		/// Asset Id not found
		AssetIdNotFound,
		/// No permission
		NoPermission,
		/// No available collection id
		NoAvailableCollectionId,
		/// Collection id does not exist
		CollectionDoesNotExist,
		/// Class Id not found
		ClassIdNotFound,
		/// Non Transferable
		NonTransferable,
		/// Invalid quantity
		InvalidQuantity,
		/// No available asset id
		NoAvailableAssetId,
		/// Asset Id is already exist
		AssetIdAlreadyExist,
		/// Asset Id is currently in an auction
		AssetAlreadyInAuction,
		/// Sign your own Asset
		SignOwnAsset,
		/// Exceed maximum batch transfer
		ExceedMaximumBatchTransfer,
		/// Exceed maximum batch minting
		ExceedMaximumBatchMinting,
		/// Exceed maximum length metadata
		ExceedMaximumMetadataLength,
		/// Error when signing support
		EmptySupporters,
		/// Insufficient Balance
		InsufficientBalance,
		/// Time-capsule executed too early
		TimecapsuleExecutedTooEarly,
		/// Only Time capsule collection
		OnlyForTimeCapsuleCollectionType,
		/// Timecapsule execution logic is invalid
		TimeCapsuleExecutionLogicIsInvalid,
		/// Timecapsule scheduled error
		ErrorWhenScheduledTimeCapsule,
		/// Collection already locked
		CollectionAlreadyLocked,
		/// Collection is locked
		CollectionIsLocked,
		/// Collection is not locked
		CollectionIsNotLocked,
		/// NFT Royalty fee exceed 50%
		RoyaltyFeeExceedLimit,
		/// NFT Asset is locked e.g on marketplace, or other locks
		AssetIsLocked,
		/// NFT mint limit is exceeded
		ExceededMintingLimit,
		/// The total amount of minted NFTs is more than the proposed hard limit
		TotalMintedAssetsForClassExceededProposedLimit,
		/// Hard limit is already set
		HardLimitIsAlreadySet,
		/// Extrisic is called using invalid NFT type
		InvalidAssetType,
		/// Invalid stackable NFT transfer (stored value is equal to zero)
		InvalidStackableNftTransfer,
		/// Invalid stackable NFT amount
		InvalidStackableNftAmount,
		/// Invalid current total issuance
		InvalidCurrentTotalIssuance,
		/// Wrong signature
		WrongSignature,
		/// Signature expired
		SignatureExpired,
		/// Fail to mint new proxy nft
		FailToMintProxyNft,
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Create a new NFT group collection from provided name and properties
		/// as NFT metadata
		///
		/// The dispatch origin for this call must be _Root_.
		/// - `name`: name of the group collection as NFT metadata
		/// - `properties`: properties of the group collection as NFT metadata
		///
		/// Emits `NewNftCollectionCreated` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::create_group())]
		#[transactional]
		pub fn create_group(
			origin: OriginFor<T>,
			name: NftMetadata,
			properties: NftMetadata,
		) -> DispatchResultWithPostInfo {
			ensure_root(origin)?;

			ensure!(
				name.len() as u32 <= T::MaxMetadata::get() && properties.len() as u32 <= T::MaxMetadata::get(),
				Error::<T>::ExceedMaximumMetadataLength
			);

			let next_group_collection_id = Self::do_create_group_collection(name.clone(), properties.clone())?;

			let collection_data = NftGroupCollectionData { name, properties };

			GroupCollections::<T>::insert(next_group_collection_id, collection_data);

			let all_collection_count = Self::all_nft_collection_count();
			let new_all_nft_collection_count = all_collection_count
				.checked_add(One::one())
				.ok_or("Overflow adding a new collection to total collection")?;

			AllNftGroupCollection::<T>::set(new_all_nft_collection_count);

			Self::deposit_event(Event::<T>::NewNftCollectionCreated(next_group_collection_id));
			Ok(().into())
		}

		/// Create new NFT class using provided NFT class data details
		///
		/// The dispatch origin for this call must be _Signed_.
		/// - `metadata`: class metadata as NFT metadata
		/// - `attributes`: class' attributes
		/// - `collection`: the class' group collection ID
		/// - `token_type`: the type of token which will be minted for this class
		/// - `collection_type`: the type of collection the class will be
		/// - `royalty_fee` - the fee (as a percent value) which will go to the class owner
		///
		/// Emits `NewNftClassCreated` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::create_class())]
		#[transactional]
		pub fn create_class(
			origin: OriginFor<T>,
			metadata: NftMetadata,
			attributes: Attributes,
			collection_id: GroupCollectionId,
			token_type: TokenType,
			collection_type: CollectionType,
			royalty_fee: Perbill,
			mint_limit: Option<u32>,
		) -> DispatchResultWithPostInfo {
			let sender = ensure_signed(origin)?;
			let _class_id = Self::do_create_class(
				&sender,
				metadata,
				attributes,
				collection_id,
				token_type,
				collection_type,
				royalty_fee,
				mint_limit,
			)?;
			Ok(().into())
		}

		/// Minting new NFTs using provided class ID, metadata,
		/// attributes, and quantity
		///
		/// The dispatch origin for this call must be _Signed_.
		/// - `class_id`: class ID of the collection the NFT will be part of
		/// - `metadata`: NFT assets metadata as NFT metadata
		/// - `attributes`: NFTs' attributes
		/// - `quantity`: the number of NFTs to be minted
		///
		/// Emits `NewNftMinted` if successful.
		#[pallet::weight(< T as Config >::WeightInfo::mint() * * quantity as u64)]
		#[transactional]
		pub fn mint(
			origin: OriginFor<T>,
			class_id: ClassIdOf<T>,
			metadata: NftMetadata,
			attributes: Attributes,
			quantity: u32,
		) -> DispatchResultWithPostInfo {
			let sender = ensure_signed(origin)?;

			ensure!(quantity >= 1, Error::<T>::InvalidQuantity);

			Self::do_mint_nfts(&sender, class_id, metadata, attributes, false, quantity)?;

			Ok(().into())
		}

		/// Minting new stackable NFTs using provided class ID, metadata,
		/// attributes, and amount
		///
		/// The dispatch origin for this call must be _Signed_.
		/// - `class_id`: class ID of the collection the NFT will be part of
		/// - `metadata`: NFT assets metadata as NFT metadata
		/// - `attributes`: NFTs' attributes
		/// - `amount`: the balance of the minted stackable NFTs
		///
		/// Emits `NewStackableNftMinted` if successful.
		#[pallet::weight(< T as Config >::WeightInfo::mint_stackable_nft())]
		#[transactional]
		pub fn mint_stackable_nft(
			origin: OriginFor<T>,
			class_id: ClassIdOf<T>,
			metadata: NftMetadata,
			attributes: Attributes,
			amount: BalanceOf<T>,
		) -> DispatchResultWithPostInfo {
			let sender = ensure_signed(origin)?;

			Self::update_class_total_issuance(&sender, &class_id, 1u32, false)?;
			// Collect minting deposit
			let class_fund: T::AccountId = T::Treasury::get().into_account_truncating();
			let deposit = T::AssetMintingFee::get().saturating_mul(Into::<BalanceOf<T>>::into(1u32));
			<T as orml_nft::Config>::Currency::transfer(
				&sender,
				&class_fund,
				deposit,
				ExistenceRequirement::KeepAlive,
			)?;

			let new_stackable_nft_data = NftAssetData {
				deposit,
				attributes: attributes,
				is_locked: true,
			};

			let result =
				NftModule::<T>::mint_stackable_nft(&sender, class_id, metadata, new_stackable_nft_data, amount);
			match result {
				Ok((token_id, _balance)) => {
					Self::deposit_event(Event::<T>::NewStackableNftMinted(sender, class_id, token_id, amount));
					Ok(().into())
				}
				Err(_) => Err(Error::<T>::InvalidStackableNftAmount.into()),
			}
		}

		/// Transfer an existing NFT asset if it is not listed in an auction
		///
		/// The dispatch origin for this call must be _Signed_.
		/// - `to`: account to transfer the NFT asset to
		/// - `asset_id`: the asset (class ID, token ID) that will be transferred
		///
		/// Emits `TransferedNft` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::transfer())]
		#[transactional]
		pub fn transfer(
			origin: OriginFor<T>,
			to: T::AccountId,
			asset_id: (ClassIdOf<T>, TokenIdOf<T>),
		) -> DispatchResultWithPostInfo {
			let sender = ensure_signed(origin)?;

			ensure!(
				Self::check_item_on_listing(asset_id.0, asset_id.1)? == false,
				Error::<T>::AssetAlreadyInAuction
			);

			Self::do_transfer(sender, to, asset_id)?;

			Ok(().into())
		}

		/// Transfer an existing NFT asset if it is not listed in an auction
		///
		/// The dispatch origin for this call must be _Signed_.
		/// - `to`: account to transfer the NFT asset to
		/// - `asset_id`: the asset (class ID, token ID) that will be transferred
		///
		/// Emits `TransferedStakcableNft` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::transfer_stackable_nft())]
		#[transactional]
		pub fn transfer_stackable_nft(
			origin: OriginFor<T>,
			to: T::AccountId,
			asset_id: (ClassIdOf<T>, TokenIdOf<T>),
			amount: BalanceOf<T>,
		) -> DispatchResultWithPostInfo {
			let sender = ensure_signed(origin)?;

			ensure!(
				Self::check_item_on_listing(asset_id.0, asset_id.1)? == false,
				Error::<T>::AssetAlreadyInAuction
			);

			ensure!(
				amount <= Self::get_free_stackable_nft_balance(&sender, &asset_id),
				Error::<T>::InvalidStackableNftTransfer
			);

			let transfer_result = NftModule::<T>::transfer_stackable_nft(&sender, &to, asset_id, amount);

			ensure!(transfer_result.is_ok(), Error::<T>::InvalidStackableNftTransfer);

			Self::deposit_event(Event::<T>::TransferedStackableNft(sender, to, asset_id, amount.into()));
			Ok(().into())
		}

		/// Transfer a batch of existing NFT assets if the batch size no more
		/// than the max batch transfer size and the asset are owned by the sender
		///
		/// The dispatch origin for this call must be _Signed_.
		/// - `to`: account to transfer the NFT asset to
		/// - `tos`: list of assets (class ID, token ID) that will be transferred
		///
		/// Emits `TransferedNft` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::transfer_batch() * tos.len() as u64)]
		#[transactional]
		pub fn transfer_batch(
			origin: OriginFor<T>,
			tos: Vec<(T::AccountId, (ClassIdOf<T>, TokenIdOf<T>))>,
		) -> DispatchResultWithPostInfo {
			let sender = ensure_signed(origin)?;

			ensure!(
				tos.len() as u32 <= T::MaxBatchTransfer::get(),
				Error::<T>::ExceedMaximumBatchTransfer
			);

			for (_i, x) in tos.iter().enumerate() {
				let item = x.clone();
				let owner = sender.clone();

				ensure!(
					Self::check_item_on_listing(item.1 .0, item.1 .1)? == false,
					Error::<T>::AssetAlreadyInAuction
				);

				Self::do_transfer(owner, item.0, (item.1 .0, item.1 .1))?;
			}

			Ok(().into())
		}

		/// Support an NFT asset with provided contribution amount if not the asset owner
		///
		/// The dispatch origin for this call must be _Signed_.
		/// - `asset_id`: the asset (class ID, token ID) that will be signed
		/// - `contribution`: the amount the sender contributes to the Nft
		///
		/// Emits no event if successful.
		#[pallet::weight(<T as Config>::WeightInfo::sign_asset())]
		#[transactional]
		pub fn sign_asset(
			origin: OriginFor<T>,
			asset_id: (ClassIdOf<T>, TokenIdOf<T>),
			contribution: BalanceOf<T>,
		) -> DispatchResultWithPostInfo {
			let sender = ensure_signed(origin)?;

			let token_info = NftModule::<T>::tokens(asset_id.0, asset_id.1).ok_or(Error::<T>::AssetInfoNotFound)?;

			ensure!(token_info.owner != sender, Error::<T>::SignOwnAsset);

			// Add contribution into class fund
			let class_fund = Self::get_class_fund(&asset_id.0);

			ensure!(
				<T as orml_nft::Config>::Currency::free_balance(&sender) > contribution,
				Error::<T>::InsufficientBalance
			);
			// Transfer contribution to class fund pot
			<T as orml_nft::Config>::Currency::transfer(
				&sender,
				&class_fund,
				contribution,
				ExistenceRequirement::KeepAlive,
			)?;

			let network_treasury = T::Treasury::get().into_account_truncating();
			// Transfer storage deposit fee
			<T as orml_nft::Config>::Currency::transfer(
				&sender,
				&network_treasury,
				T::StorageDepositFee::get().into(),
				ExistenceRequirement::KeepAlive,
			)?;

			if AssetSupporters::<T>::contains_key(&asset_id) {
				AssetSupporters::<T>::try_mutate(asset_id, |supporters| -> DispatchResult {
					let supporters = supporters.as_mut().ok_or(Error::<T>::EmptySupporters)?;
					supporters.push(sender);
					Ok(())
				})?;
			} else {
				let mut new_supporters = Vec::new();
				new_supporters.push(sender);
				AssetSupporters::<T>::insert(asset_id, new_supporters);
			}
			Ok(().into())
		}

		/// Change NFT minting promotion status to the provided value
		///
		/// The dispatch origin for this call must be _Root_.
		/// - `enable`: the promotion status (on or off)
		///
		/// Emits `PromotionEnabled` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::sign_asset())]
		#[transactional]
		pub fn enable_promotion(origin: OriginFor<T>, enable: bool) -> DispatchResultWithPostInfo {
			ensure_root(origin)?;

			PromotionEnabled::<T>::put(enable);

			Self::deposit_event(Event::<T>::PromotionEnabled(enable));
			Ok(().into())
		}

		/// Destroys NFT asset if the sender owns it
		///
		/// The dispatch origin for this call must be _Signed_.
		/// - `asset_id`: the asset (class ID, token ID) that will be burned
		///
		/// Emits `CollectionLocked` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::sign_asset())]
		#[transactional]
		pub fn burn(origin: OriginFor<T>, asset_id: (ClassIdOf<T>, TokenIdOf<T>)) -> DispatchResultWithPostInfo {
			let sender = ensure_signed(origin)?;
			ensure!(!Self::is_stackable(asset_id)?, Error::<T>::InvalidAssetType);
			Self::do_burn(&sender, &asset_id)?;
			Self::deposit_event(Event::<T>::BurnedNft(asset_id));
			Ok(().into())
		}

		/// Lock the provided collection by governance if it is not already locked
		///
		/// The dispatch origin for this call must be _Root_.
		/// - `class_id`: the class ID of the collection
		///
		/// Emits `CollectionLocked` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::sign_asset())]
		pub fn force_lock_collection(origin: OriginFor<T>, class_id: ClassIdOf<T>) -> DispatchResult {
			ensure_root(origin)?;

			ensure!(
				!LockedCollection::<T>::contains_key(class_id),
				Error::<T>::CollectionAlreadyLocked
			);

			LockedCollection::<T>::insert(class_id.clone(), ());
			Self::deposit_event(Event::<T>::CollectionLocked(class_id));

			Ok(())
		}

		/// Unlock the provided collection by governance if already locked
		///
		/// The dispatch origin for this call must be _Root_.
		/// - `class_id`: the class ID of the collection
		///
		/// Emits `CollectionUnlocked` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::sign_asset())]
		pub fn force_unlock_collection(origin: OriginFor<T>, class_id: ClassIdOf<T>) -> DispatchResult {
			ensure_root(origin)?;

			ensure!(
				LockedCollection::<T>::contains_key(class_id),
				Error::<T>::CollectionIsNotLocked
			);

			LockedCollection::<T>::remove(class_id.clone());
			Self::deposit_event(Event::<T>::CollectionUnlocked(class_id));

			Ok(())
		}

		/// Transfer a NFT asset triggered by governance
		///
		/// The dispatch origin for this call must be _Root_.
		/// - `to`: account to transfer the NFT asset to
		/// - `asset_id`: the asset (class ID, token ID) that will be transferred
		///
		/// Emits `ForceTransferredNft` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::transfer())]
		#[transactional]
		pub fn force_transfer(
			origin: OriginFor<T>,
			from: T::AccountId,
			to: T::AccountId,
			asset_id: (ClassIdOf<T>, TokenIdOf<T>),
		) -> DispatchResultWithPostInfo {
			ensure_root(origin)?;
			ensure!(
				Self::check_item_on_listing(asset_id.0, asset_id.1)? == false,
				Error::<T>::AssetAlreadyInAuction
			);

			let token_id = Self::do_force_transfer(&from, &to, asset_id)?;

			Self::deposit_event(Event::<T>::ForceTransferredNft(from, to, token_id, asset_id.clone()));

			Ok(().into())
		}

		/// Set hard limit of minted tokens for a NFT class.
		///
		/// The dispatch origin for this call must be _Signed_.
		/// Only class owner can make this call.
		/// - `class_id`: the class ID of the collection
		///
		/// Emits `HardLimitSet` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::set_hard_limit())]
		#[transactional]
		pub fn set_hard_limit(
			origin: OriginFor<T>,
			class_id: ClassIdOf<T>,
			hard_limit: u32,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;
			Classes::<T>::try_mutate(class_id, |class_info| -> DispatchResultWithPostInfo {
				let info = class_info.as_mut().ok_or(Error::<T>::ClassIdNotFound)?;

				ensure!(who.clone() == info.owner, Error::<T>::NoPermission);
				ensure!(info.data.mint_limit == None, Error::<T>::HardLimitIsAlreadySet);
				ensure!(
					info.data.total_minted_tokens <= hard_limit,
					Error::<T>::TotalMintedAssetsForClassExceededProposedLimit
				);

				info.data.mint_limit = Some(hard_limit);
				Self::deposit_event(Event::<T>::HardLimitSet(class_id));

				Ok(().into())
			})
		}

		/// Withdraws funds from class fund
		///
		/// The dispatch origin for this call must be _Signed_.
		/// Only class owner can withdraw funds.
		/// - `class_id`: the class ID of the class which funds will be withdrawn
		///
		/// Emits `ClassFundsWithdrawn` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::withdraw_funds_from_class_fund())]
		#[transactional]
		pub fn withdraw_funds_from_class_fund(
			origin: OriginFor<T>,
			class_id: ClassIdOf<T>,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;
			let class_info = NftModule::<T>::classes(class_id).ok_or(Error::<T>::ClassIdNotFound)?;

			ensure!(who.clone() == class_info.owner, Error::<T>::NoPermission);
			let class_fund_account = Self::get_class_fund(&class_id);

			// Balance minus existential deposit
			let class_fund_balance = <T as Config>::Currency::free_balance(&class_fund_account)
				.checked_sub(&<T as Config>::Currency::minimum_balance())
				.ok_or(ArithmeticError::Underflow)?;
			<T as Config>::Currency::transfer(
				&class_fund_account,
				&who,
				class_fund_balance,
				ExistenceRequirement::KeepAlive,
			)?;

			Self::deposit_event(Event::<T>::ClassFundsWithdrawn(class_id));

			Ok(().into())
		}

		/// Unlock the provided NFT by governance if already locked
		///
		/// The dispatch origin for this call must be _Root_.
		/// - `token_id`: the class ID nad token ID of the asset
		///
		/// Emits `NftUnlocked` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::sign_asset())]
		pub fn force_unlock_nft(origin: OriginFor<T>, token_id: (ClassIdOf<T>, TokenIdOf<T>)) -> DispatchResult {
			ensure_root(origin)?;

			Tokens::<T>::try_mutate_exists(&token_id.0, &token_id.1, |maybe_token_info| -> DispatchResult {
				let token_info_result = maybe_token_info.as_mut().ok_or(Error::<T>::AssetInfoNotFound)?;
				token_info_result.data.is_locked = false;
				Self::deposit_event(Event::<T>::NftUnlocked(token_id.0, token_id.1));

				Ok(())
			})
		}

		/// Force update the total issuance of a given class
		///
		/// The dispatch origin for this call must be _Root_.
		/// - `class_id`: the class ID of the collection
		/// - `current_total_issuance`: the current total issuance of the collection
		/// - `new_total_issuance`: the new total issuance of the collection
		///
		/// Emits `ClassTotalIssuanceUpdated` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::force_update_total_issuance())]
		pub fn force_update_total_issuance(
			origin: OriginFor<T>,
			class_id: ClassIdOf<T>,
			current_total_issuance: TokenIdOf<T>,
			new_total_issuance: TokenIdOf<T>,
		) -> DispatchResult {
			ensure_root(origin)?;

			// update class total issuance
			Classes::<T>::try_mutate(class_id.clone(), |class_info| -> DispatchResult {
				let info = class_info.as_mut().ok_or(Error::<T>::ClassIdNotFound)?;
				ensure!(
					current_total_issuance == info.total_issuance,
					Error::<T>::InvalidCurrentTotalIssuance
				);

				info.total_issuance = new_total_issuance;
				Self::deposit_event(Event::<T>::ClassTotalIssuanceUpdated(class_id, new_total_issuance));

				Ok(())
			})
		}

		/// Force update royalty fee of a given class
		///
		/// The dispatch origin for this call must be _Root_.
		/// - `class_id`: the class ID of the collection
		/// - `new_royalty_fee: the new royalty fee of the collection
		///
		/// Emits `ClassRoyaltyFeeUpdated` if successful.
		#[pallet::weight(<T as Config>::WeightInfo::force_update_total_issuance())]
		pub fn force_update_royalty_fee(
			origin: OriginFor<T>,
			class_id: ClassIdOf<T>,
			new_royalty_fee: Perbill,
		) -> DispatchResult {
			ensure_root(origin)?;

			ensure!(
				new_royalty_fee <= Perbill::from_percent(25u32),
				Error::<T>::RoyaltyFeeExceedLimit
			);
			// update class total issuance
			Classes::<T>::try_mutate(class_id.clone(), |class_info| -> DispatchResult {
				let info = class_info.as_mut().ok_or(Error::<T>::ClassIdNotFound)?;

				info.data.royalty_fee = new_royalty_fee;
				Self::deposit_event(Event::<T>::ClassRoyaltyFeeUpdated(class_id, new_royalty_fee));

				Ok(())
			})
		}

		/// Mint an item by providing the pre-signed approval.
		///
		/// Origin must be Signed.
		///
		/// - `mint_data`: The pre-signed approval that consists of the information about the item,
		///   its metadata, attributes, who can mint it (`None` for anyone) and until what block
		///   number.
		/// - `signature`: The signature of the `data` object.
		/// - `signer`: The `data` object's signer. Should be an Issuer of the collection.
		#[pallet::weight(<T as Config>::WeightInfo::mint_pre_signed())]
		pub fn mint_pre_signed(
			origin: OriginFor<T>,
			mint_data: Box<PreSignedMintOf<T>>,
			signature: T::OffchainSignature,
			signer: T::AccountId,
		) -> DispatchResult {
			let origin = ensure_signed(origin)?;
			Self::validate_signature(&Encode::encode(&mint_data), &signature, &signer)?;
			Self::do_mint_pre_signed(origin, *mint_data, signer)
		}

		/// Mint nft as wallet then owner as proxy acount to control this wallet.
		///
		/// Origin must be Signed.
		///
		/// The dispatch origin for this call must be _Signed_.
		/// - `class_id`: class ID of the collection the NFT will be part of
		/// - `mint_to`: address that will receive NFT
		/// - `metadata`: NFT assets metadata as NFT metadata
		/// - `attributes`: NFTs' attributes
		/// - `quantity`: the number of NFTs to be minted
		///
		/// Emits `NewNftMinted` if successful.
		#[pallet::weight(< T as Config >::WeightInfo::mint())]
		#[transactional]
		pub fn mint_nft_proxy(
			origin: OriginFor<T>,
			mint_to: T::AccountId,
			class_id: ClassIdOf<T>,
			metadata: NftMetadata,
			attributes: Attributes,
		) -> DispatchResultWithPostInfo {
			let sender = ensure_signed(origin)?;

			Self::do_mint_nft_proxy(&sender, &mint_to, class_id, None, metadata, attributes, false)?;

			Ok(().into())
		}
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		//		fn on_runtime_upgrade() -> Weight {
		//			Self::storage_migration_fix_locking_issue();
		//			Weight::from_parts(0, 0)
		//		}
	}
}

impl<T: Config> Pallet<T> {
	/// Check if promotion is enabled
	pub fn is_promotion_enabled() -> bool {
		Self::get_promotion_enabled()
	}

	/// Getting a class fund
	pub fn get_class_fund(class_id: &ClassIdOf<T>) -> T::AccountId {
		T::PalletId::get().into_sub_account_truncating(class_id)
	}

	/// Internal creation of group collection
	fn do_create_group_collection(name: Vec<u8>, properties: Vec<u8>) -> Result<GroupCollectionId, DispatchError> {
		let next_group_collection_id =
			NextGroupCollectionId::<T>::try_mutate(|collection_id| -> Result<GroupCollectionId, DispatchError> {
				let current_id = *collection_id;

				*collection_id = collection_id
					.checked_add(One::one())
					.ok_or(Error::<T>::NoAvailableCollectionId)?;

				Ok(current_id)
			})?;

		let collection_data = NftGroupCollectionData { name, properties };

		<GroupCollections<T>>::insert(next_group_collection_id, collection_data);

		Ok(next_group_collection_id)
	}

	/// Transfer an NFT
	pub fn do_transfer(
		sender: T::AccountId,
		to: T::AccountId,
		asset_id: (ClassIdOf<T>, TokenIdOf<T>),
	) -> Result<<T as orml_nft::Config>::TokenId, DispatchError> {
		ensure!(!Self::is_collection_locked(&asset_id.0), Error::<T>::CollectionIsLocked);

		ensure!(!Self::is_stackable(asset_id)?, Error::<T>::InvalidAssetType);

		let class_info = NftModule::<T>::classes(asset_id.0).ok_or(Error::<T>::ClassIdNotFound)?;
		let data = class_info.data;
		let token_info = NftModule::<T>::tokens(asset_id.0, asset_id.1).ok_or(Error::<T>::AssetInfoNotFound)?;

		ensure!(!token_info.data.is_locked, Error::<T>::AssetIsLocked);

		match data.token_type {
			TokenType::Transferable => {
				let check_ownership = Self::check_nft_ownership(&sender, &asset_id)?;
				ensure!(check_ownership, Error::<T>::NoPermission);

				NftModule::<T>::transfer(&sender, &to, asset_id.clone())?;

				Self::deposit_event(Event::<T>::TransferedNft(
					sender.clone(),
					to.clone(),
					asset_id.1,
					asset_id.clone(),
				));
				Ok(asset_id.1)
			}
			// Only allowed collection owner to transfer
			TokenType::BoundToAddress => {
				ensure!(class_info.owner == sender, Error::<T>::NonTransferable);
				NftModule::<T>::transfer(&sender, &to, asset_id.clone())?;
				Self::deposit_event(Event::<T>::TransferedNft(
					sender.clone(),
					to.clone(),
					asset_id.1,
					asset_id.clone(),
				));
				Ok(asset_id.1)
			}
		}
	}

	/// Check if account owns an NFT
	pub fn check_nft_ownership(
		sender: &T::AccountId,
		asset_id: &(ClassIdOf<T>, TokenIdOf<T>),
	) -> Result<bool, DispatchError> {
		let asset_info = NftModule::<T>::tokens(asset_id.0, asset_id.1).ok_or(Error::<T>::AssetInfoNotFound)?;
		if sender == &asset_info.owner {
			return Ok(true);
		}

		return Ok(false);
	}

	/// Check if the NFT collection is locked
	pub fn is_collection_locked(class_id: &ClassIdOf<T>) -> bool {
		let is_locked = LockedCollection::<T>::get(class_id).is_some();
		return is_locked;
	}

	/// Internal force transfer NFT only for governance override action
	fn do_force_transfer(
		sender: &T::AccountId,
		to: &T::AccountId,
		asset_id: (ClassIdOf<T>, TokenIdOf<T>),
	) -> Result<<T as orml_nft::Config>::TokenId, DispatchError> {
		ensure!(!Self::is_collection_locked(&asset_id.0), Error::<T>::CollectionIsLocked);
		ensure!(!Self::is_stackable(asset_id)?, Error::<T>::InvalidAssetType);

		NftModule::<T>::transfer(&sender, &to, asset_id.clone())?;
		Ok(asset_id.1)
	}

	/// Internal NFT minting
	fn do_mint_nfts(
		sender: &T::AccountId,
		class_id: ClassIdOf<T>,
		metadata: NftMetadata,
		attributes: Attributes,
		are_locked: bool,
		quantity: u32,
	) -> Result<(Vec<(ClassIdOf<T>, TokenIdOf<T>)>, TokenIdOf<T>), DispatchError> {
		ensure!(!Self::is_collection_locked(&class_id), Error::<T>::CollectionIsLocked);
		ensure!(
			quantity <= T::MaxBatchMinting::get(),
			Error::<T>::ExceedMaximumBatchMinting
		);
		ensure!(
			metadata.len() as u32 <= T::MaxMetadata::get(),
			Error::<T>::ExceedMaximumMetadataLength
		);

		// Update class total issuance
		Self::update_class_total_issuance(&sender, &class_id, quantity, false)?;

		let class_fund: T::AccountId = T::Treasury::get().into_account_truncating();
		let deposit = T::AssetMintingFee::get().saturating_mul(Into::<BalanceOf<T>>::into(quantity));
		<T as orml_nft::Config>::Currency::transfer(&sender, &class_fund, deposit, ExistenceRequirement::KeepAlive)?;

		let new_nft_data = NftAssetData {
			deposit,
			attributes: attributes,
			is_locked: are_locked,
		};

		let mut new_asset_ids: Vec<(ClassIdOf<T>, TokenIdOf<T>)> = Vec::new();
		let mut last_token_id: TokenIdOf<T> = Default::default();

		for _ in 0..quantity {
			let token_id = NftModule::<T>::mint(&sender, class_id, metadata.clone(), new_nft_data.clone())?;
			new_asset_ids.push((class_id, token_id));

			last_token_id = token_id;
		}

		let first_token_id = last_token_id.saturating_sub(quantity.into());

		Self::deposit_event(Event::<T>::NewNftMinted(
			(class_id, first_token_id),
			(class_id, last_token_id),
			sender.clone(),
			class_id,
			quantity,
			last_token_id,
		));

		Ok((new_asset_ids, last_token_id))
	}

	/// Internal NFT minting with token id
	fn do_mint_nft_with_token_id(
		sender: &T::AccountId,
		mint_to: &T::AccountId,
		class_id: ClassIdOf<T>,
		token_id: Option<TokenIdOf<T>>,
		metadata: NftMetadata,
		attributes: Attributes,
		is_locked: bool,
		is_pre_signed_mint: bool,
	) -> Result<TokenIdOf<T>, DispatchError> {
		ensure!(!Self::is_collection_locked(&class_id), Error::<T>::CollectionIsLocked);

		ensure!(
			metadata.len() as u32 <= T::MaxMetadata::get(),
			Error::<T>::ExceedMaximumMetadataLength
		);

		// Update class total issuance
		Self::update_class_total_issuance(&sender, &class_id, 1u32, is_pre_signed_mint)?;

		let class_fund: T::AccountId = T::Treasury::get().into_account_truncating();
		let deposit = T::AssetMintingFee::get().saturating_mul(Into::<BalanceOf<T>>::into(1u32));
		<T as orml_nft::Config>::Currency::transfer(&sender, &class_fund, deposit, ExistenceRequirement::KeepAlive)?;

		let new_nft_data = NftAssetData {
			deposit,
			attributes,
			is_locked,
		};

		let mut new_token_id: TokenIdOf<T> = Default::default();

		// Mint specific token id
		if let Some(provided_token_id) = token_id {
			NftModule::<T>::mint_with_token_id(
				&mint_to,
				class_id,
				provided_token_id,
				metadata.clone(),
				new_nft_data.clone(),
			)?;
			new_token_id = provided_token_id
		} else {
			new_token_id = NftModule::<T>::mint(&mint_to, class_id, metadata.clone(), new_nft_data.clone())?;
		}

		Self::deposit_event(Event::<T>::NewNftMinted(
			(class_id, new_token_id.clone()),
			(class_id, new_token_id),
			mint_to.clone(),
			class_id,
			1u32,
			new_token_id,
		));

		Ok(new_token_id)
	}

	// Mint with pre-signed approval from collection owner
	pub(crate) fn do_mint_pre_signed(
		mint_to: T::AccountId,
		mint_data: PreSignedMintOf<T>,
		signer: T::AccountId,
	) -> DispatchResult {
		let PreSignedMint {
			class_id,
			token_id,
			attributes,
			metadata,
			only_account,
			expired,
			mint_price,
		} = mint_data;

		// If specific account recipient specified, this will make sure requirement pass
		if let Some(account) = only_account {
			ensure!(account == mint_to, Error::<T>::NoPermission);
		}

		let now = frame_system::Pallet::<T>::block_number();
		ensure!(expired >= now, Error::<T>::SignatureExpired);

		// Get class info of the collection
		let class_info = NftModule::<T>::classes(class_id).ok_or(Error::<T>::ClassIdNotFound)?;

		// Ensure signer is owner of collection
		ensure!(signer == class_info.owner, Error::<T>::NoPermission);

		// If minting price is specified, this will transfer token to collection owner.
		if let Some(price) = mint_price {
			<T as orml_nft::Config>::Currency::transfer(
				&mint_to,
				&class_info.owner,
				price,
				ExistenceRequirement::KeepAlive,
			)?;
		}

		Self::do_mint_nft_with_token_id(
			&mint_to, &mint_to, class_id, token_id, metadata, attributes, false, true,
		)?;

		Ok(())
	}

	/// Internal NFT minting with token id
	fn do_mint_nft_proxy(
		sender: &T::AccountId,
		mint_to: &T::AccountId,
		class_id: ClassIdOf<T>,
		token_id: Option<TokenIdOf<T>>,
		metadata: NftMetadata,
		attributes: Attributes,
		is_locked: bool,
	) -> Result<(ClassIdOf<T>, TokenIdOf<T>), DispatchError> {
		let minted_token_id = Self::do_mint_nft_with_token_id(
			&sender, &mint_to, class_id, token_id, metadata, attributes, is_locked, false,
		)?;
		let nft_proxy_account: T::AccountId =
			T::PalletId::get().into_sub_account_truncating((class_id, &minted_token_id));
		let proxy_deposit = <pallet_proxy::Pallet<T>>::deposit(1u32);
		// Ensure balance above ED
		let total_deposit = proxy_deposit.saturating_add(<T as pallet_proxy::Config>::Currency::minimum_balance());

		<T as pallet_proxy::Config>::Currency::transfer(&sender, &nft_proxy_account, total_deposit, KeepAlive)?;

		Self::deposit_event(Event::<T>::NewProxyNftMinted(
			(class_id, minted_token_id.clone()),
			(class_id, minted_token_id.clone()),
			mint_to.clone(),
			class_id,
			1u32,
			minted_token_id,
			nft_proxy_account,
		));

		Ok((class_id, minted_token_id))
	}

	/// A helper method to construct metadata.
	///
	/// # Errors
	///
	/// This function returns an [`IncorrectMetadata`](crate::Error::IncorrectMetadata) dispatch
	/// error if the provided metadata is too long.
	pub fn construct_metadata(metadata: Vec<u8>) -> Result<BoundedVec<u8, T::MaxMetadata>, DispatchError> {
		Ok(BoundedVec::try_from(metadata).map_err(|_| Error::<T>::ExceedMaximumMetadataLength)?)
	}

	/// Internal NFT class creation
	fn do_create_class(
		sender: &T::AccountId,
		metadata: NftMetadata,
		attributes: Attributes,
		collection_id: GroupCollectionId,
		token_type: TokenType,
		collection_type: CollectionType,
		royalty_fee: Perbill,
		mint_limit: Option<u32>,
	) -> Result<<T as orml_nft::Config>::ClassId, DispatchError> {
		ensure!(
			metadata.len() as u32 <= T::MaxMetadata::get(),
			Error::<T>::ExceedMaximumMetadataLength
		);
		let next_class_id = NftModule::<T>::next_class_id();
		ensure!(
			GroupCollections::<T>::contains_key(collection_id),
			Error::<T>::CollectionDoesNotExist
		);

		ensure!(
			royalty_fee <= Perbill::from_percent(25u32),
			Error::<T>::RoyaltyFeeExceedLimit
		);

		// Class fund
		let class_fund: T::AccountId = T::Treasury::get().into_account_truncating();

		// Secure deposit of token class owner
		let class_deposit = T::ClassMintingFee::get();
		// Transfer fund to pot
		<T as orml_nft::Config>::Currency::transfer(
			&sender,
			&class_fund,
			class_deposit,
			ExistenceRequirement::KeepAlive,
		)?;

		let class_data = NftClassData {
			deposit: class_deposit,
			token_type,
			collection_type,
			attributes,
			is_locked: false,
			royalty_fee,
			mint_limit,
			total_minted_tokens: 0u32,
		};

		NftModule::<T>::create_class(&sender, metadata, class_data)?;

		let network_treasury = T::Treasury::get().into_account_truncating();
		// Transfer storage deposit fee
		<T as orml_nft::Config>::Currency::transfer(
			sender,
			&network_treasury,
			T::StorageDepositFee::get(),
			ExistenceRequirement::KeepAlive,
		)?;
		ClassDataCollection::<T>::insert(next_class_id, collection_id);

		Self::deposit_event(Event::<T>::NewNftClassCreated(sender.clone(), next_class_id));

		Ok(next_class_id)
	}

	/// Internal NFT burning
	fn do_burn(sender: &T::AccountId, asset_id: &(ClassIdOf<T>, TokenIdOf<T>)) -> DispatchResult {
		NftModule::<T>::burn(&sender, *asset_id)?;
		Ok(())
	}

	/// Update total minted tokens for a class
	fn update_class_total_issuance(
		sender: &T::AccountId,
		class_id: &ClassIdOf<T>,
		quantity: u32,
		is_pre_signed_mint: bool,
	) -> DispatchResult {
		// update class total issuance
		Classes::<T>::try_mutate(class_id, |class_info| -> DispatchResult {
			let info = class_info.as_mut().ok_or(Error::<T>::ClassIdNotFound)?;
			if !is_pre_signed_mint {
				ensure!(info.owner == sender.clone(), Error::<T>::NoPermission);
			}
			match info.data.mint_limit {
				Some(l) => {
					ensure!(
						l >= quantity + info.data.total_minted_tokens,
						Error::<T>::ExceededMintingLimit
					);
				}
				None => {}
			}
			info.data.total_minted_tokens += quantity;
			Ok(())
		})
	}

	/// Find total amount of issued tokens for a class
	fn get_class_token_amount(class_id: &ClassIdOf<T>) -> u32 {
		let mut total_minted_tokens = 0u32;
		for _value in Tokens::<T>::iter_prefix_values(*class_id) {
			total_minted_tokens += 1;
		}
		total_minted_tokens
	}

	/// Upgrading NFT class data
	pub fn upgrade_class_data_v2() -> Weight {
		log::info!("Start upgrading nft class data v2");
		log::info!("Start upgrading nft token data v2");
		let mut num_nft_classes = 0;
		let _num_nft_tokens = 0;
		let _asset_by_owner_updates = 0;

		Classes::<T>::translate(
			|k,
			 class_info: ClassInfo<
				T::TokenId,
				T::AccountId,
				NftClassDataV1<BalanceOf<T>>,
				BoundedVec<u8, T::MaxClassMetadata>,
			>| {
				num_nft_classes += 1;
				log::info!("Upgrading class data");
				log::info!("Class id {:?}", k);

				let total_minted_tokens_for_a_class = Self::get_class_token_amount(&k);

				let new_data = NftClassData {
					deposit: class_info.data.deposit,
					attributes: class_info.data.attributes,
					token_type: class_info.data.token_type,
					collection_type: class_info.data.collection_type,
					is_locked: class_info.data.is_locked,
					royalty_fee: class_info.data.royalty_fee,
					mint_limit: None,
					total_minted_tokens: total_minted_tokens_for_a_class,
				};

				let v: ClassInfoOf<T> = ClassInfo {
					metadata: class_info.metadata,
					total_issuance: class_info.total_issuance,
					owner: class_info.owner,
					data: new_data,
				};
				Some(v)
			},
		);

		log::info!("Classes upgraded: {}", num_nft_classes);
		Weight::from_parts(0, 0)
	}

	/// Upgrading lock of each nft
	pub fn storage_migration_fix_locking_issue() -> Weight {
		log::info!("Start storage migration of each nft due to locking issue");
		let mut num_nft_tokens = 0;
		Tokens::<T>::translate(
			|class_id,
			 token_id,
			 token_info: TokenInfo<T::AccountId, NftAssetData<BalanceOf<T>>, TokenMetadataOf<T>>| {
				num_nft_tokens += 1;
				log::info!("Upgrading existing token data to set is_locked");
				log::info!("Class id {:?}", class_id);
				log::info!("Token id {:?}", token_id);
				let mut new_data = NftAssetData {
					deposit: token_info.data.deposit,
					attributes: token_info.data.attributes,
					is_locked: token_info.data.is_locked,
				};

				if Self::check_item_on_listing(class_id, token_id) == Ok(false) {
					new_data.is_locked = false;
				};

				let v: TokenInfoOf<T> = TokenInfo {
					metadata: token_info.metadata,
					owner: token_info.owner,
					data: new_data,
				};

				Some(v)
			},
		);
		log::info!("Tokens upgraded: {}", num_nft_tokens);
		Weight::from_parts(0, 0)
	}
}

impl<T: Config> NFTTrait<T::AccountId, BalanceOf<T>> for Pallet<T> {
	type TokenId = TokenIdOf<T>;
	type ClassId = ClassIdOf<T>;

	fn check_ownership(who: &T::AccountId, asset_id: &(Self::ClassId, Self::TokenId)) -> Result<bool, DispatchError> {
		let asset_info = NftModule::<T>::tokens(asset_id.0, asset_id.1).ok_or(Error::<T>::AssetInfoNotFound)?;

		Ok(who == &asset_info.owner)
	}

	fn get_nft_detail(asset_id: (Self::ClassId, Self::TokenId)) -> Result<NftClassData<BalanceOf<T>>, DispatchError> {
		let asset_info = NftModule::<T>::classes(asset_id.0).ok_or(Error::<T>::AssetInfoNotFound)?;

		Ok(asset_info.data)
	}

	fn get_nft_group_collection(nft_collection: &Self::ClassId) -> Result<GroupCollectionId, DispatchError> {
		let group_collection_id = ClassDataCollection::<T>::get(nft_collection);
		Ok(group_collection_id)
	}

	fn check_collection_and_class(
		collection_id: GroupCollectionId,
		class_id: Self::ClassId,
	) -> Result<bool, DispatchError> {
		ensure!(
			ClassDataCollection::<T>::contains_key(class_id),
			Error::<T>::ClassIdNotFound
		);

		let class_collection_id = ClassDataCollection::<T>::get(class_id);

		Ok(class_collection_id == collection_id)
	}

	fn is_stackable(asset_id: (Self::ClassId, Self::TokenId)) -> Result<bool, DispatchError> {
		NftModule::<T>::is_stackable(asset_id)
	}

	fn create_token_class(
		sender: &T::AccountId,
		metadata: NftMetadata,
		attributes: Attributes,
		collection_id: GroupCollectionId,
		token_type: TokenType,
		collection_type: CollectionType,
		royalty_fee: Perbill,
		mint_limit: Option<u32>,
	) -> Result<ClassId, DispatchError> {
		let class_id = Self::do_create_class(
			sender,
			metadata,
			attributes,
			collection_id,
			token_type,
			collection_type,
			royalty_fee,
			mint_limit,
		)?;
		Ok(TryInto::<ClassId>::try_into(class_id).unwrap_or_default())
	}

	fn mint_token(
		sender: &T::AccountId,
		class_id: Self::ClassId,
		metadata: NftMetadata,
		attributes: Attributes,
	) -> Result<Self::TokenId, DispatchError> {
		ensure!(!Self::is_collection_locked(&class_id), Error::<T>::CollectionIsLocked);

		ensure!(
			metadata.len() as u32 <= T::MaxMetadata::get(),
			Error::<T>::ExceedMaximumMetadataLength
		);

		let class_fund: T::AccountId = T::Treasury::get().into_account_truncating();
		let deposit = T::AssetMintingFee::get().saturating_mul(Into::<BalanceOf<T>>::into(1u32));
		<T as orml_nft::Config>::Currency::transfer(&sender, &class_fund, deposit, ExistenceRequirement::KeepAlive)?;

		let new_nft_data = NftAssetData {
			deposit,
			attributes: attributes,
			is_locked: false,
		};

		let token_id = NftModule::<T>::mint(&sender, class_id, metadata.clone(), new_nft_data.clone())?;

		Self::deposit_event(Event::<T>::NewNftMinted(
			(class_id, token_id.clone()),
			(class_id, token_id),
			sender.clone(),
			class_id,
			1u32,
			token_id,
		));

		Ok(token_id)
	}

	fn burn_nft(account: &T::AccountId, nft: &(Self::ClassId, Self::TokenId)) -> DispatchResult {
		Self::do_burn(account, nft)?;

		Ok(())
	}

	fn check_item_on_listing(class_id: Self::ClassId, token_id: Self::TokenId) -> Result<bool, DispatchError> {
		let fixed_class_id = TryInto::<ClassId>::try_into(class_id).unwrap_or_default();
		let fixed_nft_id = TryInto::<TokenId>::try_into(token_id).unwrap_or_default();

		Ok(T::AuctionHandler::check_item_in_auction(ItemId::NFT(
			fixed_class_id,
			fixed_nft_id,
		)))
	}

	fn transfer_nft(sender: &T::AccountId, to: &T::AccountId, nft: &(Self::ClassId, Self::TokenId)) -> DispatchResult {
		Self::do_transfer(sender.clone(), to.clone(), nft.clone())?;

		Ok(())
	}

	fn is_transferable(nft: &(Self::ClassId, Self::TokenId)) -> Result<bool, DispatchError> {
		let class_info = NftModule::<T>::classes(nft.0).ok_or(Error::<T>::ClassIdNotFound)?;
		let data = class_info.data;

		let token = NftModule::<T>::tokens(nft.0, nft.1).ok_or(Error::<T>::AssetInfoNotFound)?;
		let token_data = token.data;

		Ok(data.token_type.is_transferable() && !token_data.is_locked)
	}

	fn get_class_fund(class_id: &Self::ClassId) -> T::AccountId {
		T::PalletId::get().into_sub_account_truncating(class_id)
	}

	fn set_lock_collection(class_id: Self::ClassId, is_locked: bool) -> sp_runtime::DispatchResult {
		Classes::<T>::try_mutate(class_id, |class_info| -> DispatchResult {
			let info = class_info.as_mut().ok_or(Error::<T>::ClassIdNotFound)?;
			info.data.is_locked = is_locked;
			Ok(())
		})
	}

	fn set_lock_nft(token_id: (Self::ClassId, Self::TokenId), is_locked: bool) -> sp_runtime::DispatchResult {
		Tokens::<T>::try_mutate(token_id.0, token_id.1, |token_info| -> DispatchResult {
			let t = token_info.as_mut().ok_or(Error::<T>::AssetInfoNotFound)?;
			t.data.is_locked = is_locked;
			Ok(())
		})
	}

	fn get_nft_class_detail(class_id: Self::ClassId) -> Result<NftClassData<BalanceOf<T>>, DispatchError> {
		let asset_info = NftModule::<T>::classes(class_id).ok_or(Error::<T>::AssetInfoNotFound)?;
		Ok(asset_info.data)
	}

	fn get_total_issuance(class_id: Self::ClassId) -> Result<Self::TokenId, DispatchError> {
		let class_info = NftModule::<T>::classes(class_id).ok_or(Error::<T>::AssetInfoNotFound)?;
		Ok(class_info.total_issuance)
	}

	fn get_asset_owner(asset_id: &(Self::ClassId, Self::TokenId)) -> Result<T::AccountId, DispatchError> {
		let asset_info = NftModule::<T>::tokens(asset_id.0, asset_id.1).ok_or(Error::<T>::AssetInfoNotFound)?;
		Ok(asset_info.owner)
	}

	fn mint_token_with_id(
		sender: &T::AccountId,
		class_id: Self::ClassId,
		token_id: Self::TokenId,
		metadata: NftMetadata,
		attributes: Attributes,
	) -> Result<Self::TokenId, DispatchError> {
		Self::do_mint_nft_with_token_id(
			sender,
			sender,
			class_id,
			Some(token_id),
			metadata,
			attributes,
			false,
			false,
		)
	}

	fn get_free_stackable_nft_balance(who: &T::AccountId, asset_id: &(Self::ClassId, Self::TokenId)) -> BalanceOf<T> {
		let total_balance = NftModule::<T>::get_stackable_collections_balances((asset_id.0, asset_id.1, who));
		let reserved_balance = Self::reserved_stackable_nft_balances(who, asset_id);
		total_balance.saturating_sub(reserved_balance)
	}

	fn reserve_stackable_nft_balance(
		who: &T::AccountId,
		asset_id: &(Self::ClassId, Self::TokenId),
		amount: BalanceOf<T>,
	) -> sp_runtime::DispatchResult {
		let reserved_balance = Self::reserved_stackable_nft_balances(who, asset_id);
		ReservedStackableNftBalance::<T>::insert(who, asset_id, reserved_balance.saturating_add(amount));
		Ok(())
	}

	fn unreserve_stackable_nft_balance(
		who: &T::AccountId,
		asset_id: &(Self::ClassId, Self::TokenId),
		amount: BalanceOf<T>,
	) -> sp_runtime::DispatchResult {
		let reserved_balance = Self::reserved_stackable_nft_balances(who, asset_id);
		ReservedStackableNftBalance::<T>::insert(who, asset_id, reserved_balance.saturating_sub(amount));
		Ok(())
	}

	fn transfer_stackable_nft(
		sender: &T::AccountId,
		to: &T::AccountId,
		asset_id: &(Self::ClassId, Self::TokenId),
		amount: BalanceOf<T>,
	) -> DispatchResult {
		NftModule::<T>::transfer_stackable_nft(&sender, &to, *asset_id, amount);

		Ok(())
	}
}
