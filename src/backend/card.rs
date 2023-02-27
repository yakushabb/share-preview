// Copyright 2021 Rafael Mardojai CM
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    error,
    fmt::{Display, Formatter, Result as FmtResult}
};

use gettextrs::*;

use crate::vec_of_strings;
use super::{
    Data,
    Image,
    ImageError,
    Log,
    LogLevel,
    Social,
    social::{
        SocialMetaLookup,
        SocialConstraints,
        SocialImageSizeKind
    }
};

#[derive(Debug, Default, Clone)]
pub enum CardSize {
    #[default]
    Small,
    Medium,
    Large,
}

impl CardSize {
    pub fn from_social(kind: &SocialImageSizeKind) -> CardSize {
        match kind {
            SocialImageSizeKind::Small => CardSize::Small,
            SocialImageSizeKind::Medium => CardSize::Medium,
            SocialImageSizeKind::Large => CardSize::Large
        }
    }

    pub fn image_size(&self) -> (u32, u32) {
        match self {
            Self::Small => (64, 64),
            Self::Medium => (125, 125),
            Self::Large => (500, 250)
        }
    }

    pub fn icon_size(&self) -> i32 {
        match self {
            Self::Small => 32,
            Self::Medium => 48,
            Self::Large => 64
        }
    }
}

#[derive(Debug, Clone)]
pub struct Card {
    pub title: String,
    pub site: String,
    pub description: Option<String>,
    pub image: Option<Vec<u8>>,
    pub size: CardSize,
    pub social: Social,
}

impl Card {
    pub async fn new(data: &Data, social: Social, logger: &impl Log) -> Result<Card, CardError> {
        //! Create a new Card from the found metadata based on the given Social

        let lookups: SocialMetaLookup = social.lookups();
        let constraints: SocialConstraints = social.constraints();
        let mut site = data.url.clone();
        let mut size = CardSize::default(); // Default card size
        let mut image: Option<Vec<u8>> = Option::None;
        let mut image_sizes: Vec<SocialImageSizeKind> = Vec::new();

        // Prepare with already available data
        match social {
            Social::Facebook => {
                image_sizes.push(SocialImageSizeKind::Large);
                image_sizes.push(SocialImageSizeKind::Medium);
                site = site.to_uppercase();
            },
            Social::Mastodon => {
                image_sizes.push(SocialImageSizeKind::Small);
                // Mastodon uses og:site_name
                let look = vec_of_strings!["og:site_name"];
                if let Some(val) = Card::lookup_meta(&look, &data, logger) {
                    if !val.is_empty() {
                        site = val.to_string();
                        logger.log(LogLevel::Info, gettext!("{}: Found {}.", &social, "og:site_name"));
                    } else {
                        logger.log(LogLevel::Warning, gettext!("{}: {} is empty!", &social, "og:site_name"));
                    }
                } else {
                    logger.log(LogLevel::Warning, gettext!(
                        "{}: Unable to find {}. Consider providing a {} meta property.", &social, "og:site_name", "og:site_name"
                    ));
                }
            },
            Social::Twitter => {
                // Change card size by the value of "twitter:card" meta-tag
                let look = vec_of_strings!["twitter:card"];
                if let Some(val) = Card::lookup_meta(&look, &data, logger) {
                    if val == "summary_large_image" {
                        image_sizes.push(SocialImageSizeKind::Large);
                    }
                    logger.log(LogLevel::Info, gettext!("{}: Found card of type {}.", &social, val));
                } else {
                    logger.log(LogLevel::Warning, gettext!(
                        "{}: Unable to find {}. Consider providing a {} meta property.", &social, "twitter:card", "twitter:card"
                    ));
                }
                image_sizes.push(SocialImageSizeKind::Medium);
            }
        }

        // Get first available value from meta-tags to lookup
        let pre_title = Card::lookup_meta(&lookups.title, &data, logger);
        let title = match &pre_title {
            Some(title) => title.to_string(),
            None => {
                logger.log(LogLevel::Warning, gettext!(
                    "{}: Unable to find a metadata for title!. Falling back to document title.",
                    &social
                ));
                match &data.title {
                    Some(title) => {
                        title.to_string()
                    },
                    None => {
                        logger.log(LogLevel::Warning, gettext!(
                            "{}: Unable to find the document title!. Falling back to site url.",
                            &social
                        ));
                        site.to_string()
                    },
                }
            }
        };

        // TODO: Get description from HTML for Facebook
        let description = Card::lookup_meta(&lookups.description, &data, logger);

        let card_type = Card::lookup_meta(&lookups.kind, &data, logger);

        // Return error if no basic data is found for Twitter
        if let (Social::Twitter, Option::None, Option::None) = (&social, &pre_title, &description) {
            logger.log(LogLevel::Error, gettext!("{}: Unable to find any valid title or description.", &social));
            return Err(CardError::NotEnoughData);
        }
        // Return error if no card type is found for Twitter
        if let (Social::Twitter, Option::None) = (&social, &card_type) {
            logger.log(LogLevel::Error, gettext!("{}: Unable to find any valid card type.", &social));
            return Err(CardError::TwitterNoCardFound);
        }

        match Card::lookup_meta_image(
            &social,
            &lookups.image,
            &data,
            &image_sizes,
            &constraints,
            logger
        ).await {
            Some((i, s)) => {
                image = Some(i);
                size = s;
            },
            None => {
                match &social {
                    Social::Facebook => {
                        logger.log(LogLevel::Warning, gettext!(
                            "{}: No valid image found, will look for images in the HTML.", &social
                        ));
                        if data.body_images.len() > 0 {
                            match Card::lookup_fb_body_images(&social, &data.body_images, &constraints).await {
                                Some(i) => {
                                    image = Some(i);
                                    size = CardSize::Medium;
                                },
                                _ => ()
                            }
                        }
                    },
                    Social::Mastodon => {
                        logger.log(LogLevel::Warning, gettext!(
                            "{}: Unable to find a valid image in the metadata, will render an icon.", &social
                        ));
                    },
                    Social::Twitter => {
                        logger.log(LogLevel::Warning, gettext!(
                            "{}: Unable to find a valid image in the metadata, will render a \"{}\" card with icon.", &social, "summary"
                        ));
                        size = CardSize::Medium;
                    }
                }
            }
        }

        Ok(Card {title, site, description, image, size, social})
    }

    pub fn log_message() {

    }

    pub fn lookup_meta(lookup: &Vec<String>, data: &Data, logger: &impl Log) -> Option<String> {
        for name in lookup.iter() {
            let occurrences = data.get_meta(name);

            if let Some(meta) = occurrences.first() {
                if let Some(val) = &meta.content {
                    if !val.is_empty() {
                        logger.log(LogLevel::Debug, gettext!(
                            "Found a valid occurrence for: \"{}\" with value: \"{}\".",
                            name, val
                        ));
                        return Some(val.to_string());
                    } {
                        logger.log(LogLevel::Warning, gettext!("\"{}\" it's empty!", name));
                        continue;
                    }
                }
            };

            logger.log(LogLevel::Debug, gettext!("No occurrences found for \"{}\"!", name));
        }
        None
    }

    pub async fn lookup_meta_image(
        social: &Social,
        lookup: &Vec<String>,
        data: &Data,
        kinds: &Vec<SocialImageSizeKind>,
        constraints: &SocialConstraints,
        logger: &impl Log
    ) -> Option<(Vec<u8>, CardSize)> {
        for name in lookup.iter() {
            let occurrences = data.get_meta(name);

            for meta in occurrences.iter() {
                if let Some(image) = &meta.image {
                    logger.log(LogLevel::Debug, gettext!(
                        "Found a valid occurrence for: \"{}\" with value \"{}\".",
                        name, image.url
                    ));

                    match Card::try_get_image(social, image, kinds, constraints).await {
                        Ok((bytes, size)) => {
                            logger.log(LogLevel::Debug, gettext!("Processed image successfully: \"{}\".", image.url));
                            return Some((bytes, size));
                        }
                        Err(err) => {
                            logger.log(LogLevel::Error, gettext!("Failed to process image: \"{}\": {}.", image.url, err));
                            continue;
                        }
                    }
                }
            }
            logger.log(LogLevel::Debug, gettext!("No valid occurrences found for \"{}\"!", name));
        }
        None
    }

    pub async fn lookup_fb_body_images(
        social: &Social,
        images: &Vec<Image>,
        constraints: &SocialConstraints
    ) -> Option<Vec<u8>> {
        let kinds = [SocialImageSizeKind::Medium].to_vec();

        for image in images.iter() {
            match Card::try_get_image(social, &image, &kinds, constraints).await {
                Ok((bytes, _)) => {
                    return Some(bytes)
                }
                Err(_) => continue
            }
        }
        None
    }

    pub async fn try_get_image(
        social: &Social,
        image: &Image,
        kinds: &Vec<SocialImageSizeKind>,
        _constraints: &SocialConstraints
    ) -> Result<(Vec<u8>, CardSize), ImageError> {
        image.thumbnail(social, kinds).await
    }
}

#[derive(Debug)]
pub enum CardError {
    NotEnoughData,
    TwitterNoCardFound
}

impl Display for CardError {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        match *self {
            CardError::NotEnoughData => write!(f, "NotEnoughData"),
            CardError::TwitterNoCardFound => write!(f, "TwitterNoCardFound"),
        }
    }
}

impl error::Error for CardError {}
